//! Typed document model — one classified fetched resource.
//!
//! A `Document` is *metadata about* a fetched resource, never the payload
//! itself: bounded fields only (url, MIME, charset, size, checksum, title,
//! provenance). Bodies stay in the sink/object they were delivered to; nothing
//! here embeds unbounded content, so a crawl of 100k pages still fits in RAM
//! as a document index.
//!
//! The model is deliberately the join point between three subsystems:
//! * [`crate::content`] supplies the classification (`kind`, `mime`, `charset`);
//! * [`crate::crawl::discovery`] supplies provenance ([`DiscoverySource`]);
//! * [`crate::media`] carries the resource-only view of non-record content.
//!
//! [`DocumentSet`] is the bounded collection the planner fills: it caps
//! membership, de-duplicates by URL (and by content checksum when present) and
//! reports why something was refused instead of silently dropping it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::content::{ContentInfo, ContentKind};

/// How a URL/record entered the crawl. Kept on the document so a delivered
/// dataset can always answer "where did this row come from".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoverySource {
    /// A configured `start_url`.
    Start,
    /// An ordinary `<a href>` found on a crawled page.
    Link,
    /// A `rel=next/prev` (or configured selector) pagination link.
    Pagination,
    /// Enumerated from a sitemap (`<urlset>`).
    Sitemap,
    /// Enumerated from a sitemap *index* (a pointer to another sitemap).
    SitemapIndex,
    /// An RSS/Atom feed reference.
    Feed,
    /// A `rel=alternate` representation (language, format, …).
    Alternate,
    /// A `rel=canonical` target — recorded, and de-duplicated against the
    /// URL it canonicalizes.
    Canonical,
    /// A downloadable file link (PDF, CSV, XLSX, …).
    Resource,
    /// An image/audio/video resource.
    Media,
}

impl DiscoverySource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Link => "link",
            Self::Pagination => "pagination",
            Self::Sitemap => "sitemap",
            Self::SitemapIndex => "sitemap-index",
            Self::Feed => "feed",
            Self::Alternate => "alternate",
            Self::Canonical => "canonical",
            Self::Resource => "resource",
            Self::Media => "media",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "start" => Self::Start,
            "link" => Self::Link,
            "pagination" => Self::Pagination,
            "sitemap" => Self::Sitemap,
            "sitemap-index" => Self::SitemapIndex,
            "feed" => Self::Feed,
            "alternate" => Self::Alternate,
            "canonical" => Self::Canonical,
            "resource" => Self::Resource,
            "media" => Self::Media,
            _ => return None,
        })
    }
}

impl std::fmt::Display for DiscoverySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One fetched resource, described. Construction goes through
/// [`Document::observed`] so the checksum and timestamps are never forgotten.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// URL as requested (before redirects).
    pub url: String,
    /// URL after redirects, when the transport reported one.
    pub final_url: Option<String>,
    /// Target id that produced the document.
    pub target: String,
    pub kind: ContentKind,
    pub mime: String,
    pub charset: Option<String>,
    /// Body size in bytes (as delivered, post-decompression).
    pub bytes: u64,
    /// sha256 of the body, when the caller computed one.
    pub sha256: Option<String>,
    /// `<title>` or filename, when one is recoverable.
    pub title: Option<String>,
    pub status: Option<u16>,
    /// Crawl depth (0 = a start URL).
    pub depth: u32,
    /// Unix seconds.
    pub fetched_ts: i64,
    pub source: DiscoverySource,
    /// True when the declared Content-Type contradicted the sniffed body.
    pub mismatch: bool,
}

impl Document {
    /// Build a document from a URL plus everything the transport observed.
    #[allow(clippy::too_many_arguments)]
    pub fn observed(
        url: impl Into<String>,
        target: impl Into<String>,
        info: &ContentInfo,
        source: DiscoverySource,
        depth: u32,
        status: Option<u16>,
        fetched_ts: i64,
    ) -> Self {
        Self {
            url: url.into(),
            final_url: None,
            target: target.into(),
            kind: info.kind,
            mime: info.mime.clone(),
            charset: info.charset.clone(),
            bytes: info.bytes,
            sha256: None,
            title: None,
            status,
            depth,
            fetched_ts,
            source,
            mismatch: info.mismatch,
        }
    }

    pub fn with_sha256(mut self, sha: Option<String>) -> Self {
        self.sha256 = sha;
        self
    }

    pub fn with_title(mut self, title: Option<String>) -> Self {
        self.title = title;
        self
    }

    pub fn with_final_url(mut self, final_url: Option<String>) -> Self {
        self.final_url = final_url;
        self
    }

    /// Records are only produced from kinds we can actually parse; PDFs and
    /// binaries are delivered as *resources* (see `crate::media`).
    pub fn is_record_source(&self) -> bool {
        self.kind.is_textual()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "url": self.url,
            "final_url": self.final_url,
            "target": self.target,
            "kind": self.kind.as_str(),
            "mime": self.mime,
            "charset": self.charset,
            "bytes": self.bytes,
            "sha256": self.sha256,
            "title": self.title,
            "status": self.status,
            "depth": self.depth,
            "fetched_ts": self.fetched_ts,
            "source": self.source.as_str(),
            "mismatch": self.mismatch,
        })
    }
}

/// Why a document was not added to a [`DocumentSet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Added,
    /// Same URL already present (after fragment stripping + normalization).
    Duplicate,
    /// Same body checksum already present under another URL.
    DuplicateContent,
    /// The set is at its configured cap.
    Capped,
}

/// A bounded, de-duplicated document index.
#[derive(Debug, Clone)]
pub struct DocumentSet {
    items: Vec<Document>,
    seen_urls: std::collections::HashSet<String>,
    seen_digests: std::collections::HashSet<String>,
    cap: usize,
}

impl DocumentSet {
    /// `cap` is a hard ceiling: the set never grows past it (default 10_000).
    pub fn new(cap: usize) -> Self {
        Self {
            items: Vec::new(),
            seen_urls: std::collections::HashSet::new(),
            seen_digests: std::collections::HashSet::new(),
            cap,
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    pub fn push(&mut self, mut doc: Document) -> PushOutcome {
        doc.url = canonical_url_key(&doc.url);
        if doc.url.is_empty() {
            return PushOutcome::Duplicate;
        }
        if self.seen_urls.contains(&doc.url) {
            return PushOutcome::Duplicate;
        }
        if let Some(sha) = doc.sha256.as_deref() {
            if self.seen_digests.contains(sha) {
                return PushOutcome::DuplicateContent;
            }
        }
        if self.items.len() >= self.cap {
            return PushOutcome::Capped;
        }
        self.seen_urls.insert(doc.url.clone());
        if let Some(sha) = doc.sha256.clone() {
            self.seen_digests.insert(sha);
        }
        self.items.push(doc);
        PushOutcome::Added
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Document> {
        self.items.iter()
    }

    pub fn documents(&self) -> &[Document] {
        &self.items
    }

    pub fn of_kind(&self, kind: ContentKind) -> Vec<&Document> {
        self.items.iter().filter(|d| d.kind == kind).collect()
    }

    /// Records-only view, in insertion order.
    pub fn record_sources(&self) -> Vec<&Document> {
        self.items.iter().filter(|d| d.is_record_source()).collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "count": self.items.len(),
            "cap": self.cap,
            "documents": self.items.iter().map(|d| d.to_json()).collect::<Vec<_>>(),
        })
    }
}

/// Cheap URL key for de-duplication: lowercase scheme+host, drop the fragment
/// and any trailing empty query. Not a general URL normalizer — just enough to
/// stop `/a` and `/a#section` from becoming two documents.
pub fn canonical_url_key(url: &str) -> String {
    let trimmed = url.trim();
    let no_fragment = trimmed.split('#').next().unwrap_or("");
    match no_fragment.split_once("://") {
        Some((scheme, rest)) => {
            let (host, path) = match rest.find('/') {
                Some(i) => (&rest[..i], &rest[i..]),
                None => (rest, ""),
            };
            let mut out = format!(
                "{}://{}",
                scheme.to_ascii_lowercase(),
                host.to_ascii_lowercase()
            );
            out.push_str(path);
            out.strip_suffix('?').unwrap_or(&out).to_string()
        }
        None => no_fragment.to_string(),
    }
}

/// Extract `<title>` text from HTML without pulling in a parser pass: the
/// first `<title>…</title>` pair wins, entities are left alone (titles are
/// metadata, not data), and the result is whitespace-collapsed and bounded.
pub fn title_from_html(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<title")?;
    let gt = lower[start..].find('>')? + start + 1;
    let end = lower[gt..].find("</title>")? + gt;
    let raw = &html[gt..end];
    let title: String = raw
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .take(256)
        .collect();
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// sha256 hex of a body. Shared by the sink, export and document paths so a
/// checksum means the same thing everywhere.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

/// Deterministic, filesystem-safe dataset id: `dataset-<id>.<ext>`. Anything
/// outside the engine's key-token alphabet becomes `_`, and an empty/unsafe id
/// degrades to `dataset` rather than producing a weird filename.
pub fn safe_dataset_name(id: &str) -> String {
    if crate::ids::is_safe_key_token(id) {
        format!("dataset-{id}")
    } else {
        let cleaned: String = id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let cleaned = cleaned.trim_matches(|c| c == '_' || c == '-').to_string();
        if cleaned.is_empty() {
            "dataset".to_string()
        } else {
            format!("dataset-{cleaned}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> ContentInfo {
        ContentInfo {
            kind: ContentKind::Html,
            mime: "text/html".to_string(),
            charset: Some("utf-8".to_string()),
            bytes: 42,
            header_mime: Some("text/html".to_string()),
            mismatch: false,
            decodable_text: true,
        }
    }

    #[test]
    fn dedupes_urls_and_checksums() {
        let mut set = DocumentSet::new(10);
        let mut a = Document::observed(
            "https://x.test/a",
            "t",
            &info(),
            DiscoverySource::Link,
            1,
            Some(200),
            0,
        );
        a = a.with_sha256(Some("deadbeef".to_string()));
        assert_eq!(set.push(a), PushOutcome::Added);
        let mut b = Document::observed(
            "https://x.test/a#frag",
            "t",
            &info(),
            DiscoverySource::Link,
            1,
            Some(200),
            0,
        );
        b = b.with_sha256(Some("cafebabe".to_string()));
        assert_eq!(set.push(b), PushOutcome::Duplicate);
        let mut c = Document::observed(
            "https://x.test/b",
            "t",
            &info(),
            DiscoverySource::Link,
            1,
            Some(200),
            0,
        );
        c = c.with_sha256(Some("deadbeef".to_string()));
        assert_eq!(set.push(c), PushOutcome::DuplicateContent);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn caps_membership() {
        let mut set = DocumentSet::new(2);
        for i in 0..3 {
            let d = Document::observed(
                format!("https://x.test/{i}"),
                "t",
                &info(),
                DiscoverySource::Link,
                0,
                None,
                0,
            );
            set.push(d);
        }
        assert_eq!(set.len(), 2);
        assert_eq!(set.cap(), 2);
    }

    #[test]
    fn titles_are_bounded_and_collapsed() {
        assert_eq!(
            title_from_html("<html><head><title>  Big   Sale \n 2026 </title></head>").as_deref(),
            Some("Big Sale 2026")
        );
        assert_eq!(title_from_html("<html><title></title>"), None);
        assert_eq!(title_from_html("no title here"), None);
    }

    #[test]
    fn dataset_names_are_safe() {
        assert_eq!(safe_dataset_name("run-7"), "dataset-run-7");
        assert_eq!(safe_dataset_name("../../etc/passwd"), "dataset-etc_passwd");
        assert_eq!(safe_dataset_name("///"), "dataset");
    }

    #[test]
    fn checksums_are_stable_hex() {
        assert_eq!(sha256_hex(b"").len(), 64);
        assert_eq!(sha256_hex(b"abc"), sha256_hex(b"abc"));
        assert_ne!(sha256_hex(b"abc"), sha256_hex(b"abd"));
    }
}
