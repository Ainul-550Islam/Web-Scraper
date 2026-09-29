//! Media & resource discovery — metadata only, never a silent download.
//!
//! When discovery finds a PDF, a CSV, an image or a video, the engine records
//! *what it is* and *where it came from*; it does not pull the bytes. That
//! distinction is the whole module:
//!
//! * a resource is admitted only if the caller already knows its size, or
//!   accepts an unknown size under the configured cap (unknown size is never
//!   treated as "small");
//! * anything over [`MediaPolicy::max_bytes`] is refused with
//!   [`PushOutcome::TooLarge`] — there is no "download it anyway" path;
//! * MIME/extension allow- and deny-lists gate collection, mirroring the
//!   content-type scope the crawl plan declares;
//! * a bounded cap stops one media-rich page from filling the index.
//!
//! Checksums are recorded when the caller has them (e.g. from a completed
//! fetch); discovery alone produces no checksum, and the field stays `None`
//! rather than being invented.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::content::{kind_from_mime, mime_from_extension};
use crate::document::DiscoverySource;

/// Which bucket a discovered resource falls into. Used for grouping in the
/// console and for the resource allow-list; it is *derived* from MIME, never
/// from the link text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceKind {
    Image,
    Audio,
    Video,
    /// A downloadable document: PDF, CSV, XLSX, DOCX, JSON, XML, …
    Document,
    /// Archive/dataset containers (zip, gzip, parquet, …).
    Archive,
    /// Type could not be established from MIME or extension.
    Unknown,
}

impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Audio => "audio",
            Self::Video => "video",
            Self::Document => "document",
            Self::Archive => "archive",
            Self::Unknown => "unknown",
        }
    }

    /// True for kinds the console offers as "collect these".
    pub fn is_collectable(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

/// One discovered resource. No payloads, no local paths — just what the
/// manifest/delivery layer needs to describe it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaResource {
    pub url: String,
    /// Canonical MIME: from the response header when known, otherwise inferred
    /// from the extension and marked as such via `mime_from_extension`.
    pub mime: String,
    pub ext: Option<String>,
    /// Size in bytes when known (HEAD, Content-Length, or a completed fetch).
    pub size: Option<u64>,
    /// sha256 when the bytes were actually seen. `None` ≠ "verified".
    pub sha256: Option<String>,
    /// Page that linked the resource.
    pub source_page: String,
    pub kind: ResourceKind,
    /// How the link was found (img/src, <a href> on a PDF, …).
    pub source: DiscoverySource,
    pub depth: u32,
    /// True when `mime` came from the URL extension rather than a header.
    pub mime_guessed: bool,
}

impl MediaResource {
    pub fn to_json(&self) -> Value {
        json!({
            "url": self.url,
            "mime": self.mime,
            "ext": self.ext,
            "size": self.size,
            "sha256": self.sha256,
            "source_page": self.source_page,
            "kind": self.kind.as_str(),
            "source": self.source.as_str(),
            "depth": self.depth,
            "mime_guessed": self.mime_guessed,
        })
    }
}

/// Why a resource was not collected. Every branch is reported by the crawl
/// plan's stats; none of them silently drop the resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushOutcome {
    Added,
    Duplicate,
    /// Over `max_bytes`, or of unknown size with `allow_unknown_size == false`.
    TooLarge,
    /// Rejected by the MIME/extension allow- or deny-list.
    NotAllowed,
    /// The collector is at `max_resources` (or `max_per_page`).
    Capped,
}

/// The bounds under which resources may be recorded. Defaults are deliberately
/// conservative: nothing is collected until a plan sets a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaPolicy {
    /// Refuse any resource larger than this. 0 disables resource collection.
    pub max_bytes: u64,
    /// Cap on the whole collection.
    pub max_resources: usize,
    /// Cap per source page (stops one gallery from eating the budget).
    pub max_per_page: usize,
    /// When empty, every MIME is allowed (subject to `deny`).
    pub allow: Vec<String>,
    /// Always refused, evaluated first.
    pub deny: Vec<String>,
    /// Accept a resource whose size is not yet known (only meaningful with a
    /// transport that enforces `max_bytes` while reading).
    pub allow_unknown_size: bool,
}

impl Default for MediaPolicy {
    fn default() -> Self {
        Self {
            max_bytes: 16 * 1024 * 1024,
            max_resources: 1_000,
            max_per_page: 100,
            allow: Vec::new(),
            deny: Vec::new(),
            allow_unknown_size: false,
        }
    }
}

impl MediaPolicy {
    /// True when no resource of any kind may be recorded.
    pub fn disabled(&self) -> bool {
        self.max_bytes == 0 || self.max_resources == 0
    }

    fn allowed(&self, mime: &str, ext: Option<&str>) -> bool {
        let haystack = mime.to_ascii_lowercase();
        if self
            .deny
            .iter()
            .any(|d| haystack.contains(&d.to_ascii_lowercase()))
        {
            return false;
        }
        if self.allow.is_empty() {
            return true;
        }
        self.allow.iter().any(|a| {
            let a = a.to_ascii_lowercase();
            haystack.contains(&a) || ext.is_some_and(|e| e == a.trim_start_matches('.'))
        })
    }
}

/// Bounded, de-duplicated resource index for one crawl.
#[derive(Debug, Clone, Default)]
pub struct MediaCollector {
    items: Vec<MediaResource>,
    seen: HashSet<String>,
    per_page: std::collections::HashMap<String, usize>,
    policy: MediaPolicy,
}

impl MediaCollector {
    pub fn new(policy: MediaPolicy) -> Self {
        Self {
            items: Vec::new(),
            seen: HashSet::new(),
            per_page: std::collections::HashMap::new(),
            policy,
        }
    }

    pub fn policy(&self) -> &MediaPolicy {
        &self.policy
    }

    pub fn resources(&self) -> &[MediaResource] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Admit a resource under the policy. Order of checks is part of the
    /// contract: deny/allow first (cheapest, and independent of transport),
    /// then size, then caps, then de-duplication.
    pub fn push(&mut self, r: MediaResource) -> PushOutcome {
        if self.policy.disabled() {
            return PushOutcome::NotAllowed;
        }
        if !self.policy.allowed(&r.mime, r.ext.as_deref()) {
            return PushOutcome::NotAllowed;
        }
        match r.size {
            Some(size) if size > self.policy.max_bytes => return PushOutcome::TooLarge,
            None if !self.policy.allow_unknown_size => return PushOutcome::TooLarge,
            _ => {}
        }
        if self.items.len() >= self.policy.max_resources {
            return PushOutcome::Capped;
        }
        let page_count = self.per_page.get(&r.source_page).copied().unwrap_or(0);
        if page_count >= self.policy.max_per_page {
            return PushOutcome::Capped;
        }
        if !self.seen.insert(r.url.clone()) {
            return PushOutcome::Duplicate;
        }
        self.per_page
            .entry(r.source_page.clone())
            .and_modify(|n| *n += 1)
            .or_insert(1);
        self.items.push(r);
        PushOutcome::Added
    }

    pub fn total_bytes(&self) -> u64 {
        self.items.iter().filter_map(|r| r.size).sum()
    }

    pub fn by_kind(&self, kind: ResourceKind) -> Vec<&MediaResource> {
        self.items.iter().filter(|r| r.kind == kind).collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "count": self.items.len(),
            "total_bytes": self.total_bytes(),
            "max_bytes": self.policy.max_bytes,
            "resources": self.items.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
        })
    }
}

/// Lowercased file extension of a URL path, when it has one. Reported for the
/// record; never used to decide how bytes are parsed (see `crate::content`).
pub fn extension_of(url: &str) -> Option<String> {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() && ext.len() <= 8 => Some(ext.to_ascii_lowercase()),
        _ => None,
    }
}

/// Classify a resource by MIME first, extension second.
pub fn resource_kind_for(mime: &str, ext: Option<&str>) -> ResourceKind {
    let mime = mime.to_ascii_lowercase();
    if let Some(kind) = kind_from_mime(&mime) {
        return match kind {
            crate::content::ContentKind::Pdf
            | crate::content::ContentKind::Csv
            | crate::content::ContentKind::Json
            | crate::content::ContentKind::Xml
            | crate::content::ContentKind::Text
            | crate::content::ContentKind::Html => ResourceKind::Document,
            _ => ResourceKind::Unknown,
        };
    }
    let top = mime.split('/').next().unwrap_or("");
    match top {
        "image" => ResourceKind::Image,
        "audio" => ResourceKind::Audio,
        "video" => ResourceKind::Video,
        "application" => match mime.as_str() {
            "application/zip" | "application/gzip" | "application/x-tar" => ResourceKind::Archive,
            "application/vnd.apache.parquet" => ResourceKind::Archive,
            "application/msword"
            | "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
                ResourceKind::Document
            }
            _ => match ext.and_then(mime_from_extension) {
                Some(m) => resource_kind_for(m, None),
                None => ResourceKind::Unknown,
            },
        },
        _ => match ext.and_then(mime_from_extension) {
            Some(m) => resource_kind_for(m, None),
            None => ResourceKind::Unknown,
        },
    }
}

/// Build a resource record with MIME resolution in one place: a real header
/// beats a guessed extension every time, and guesses are flagged.
#[allow(clippy::too_many_arguments)]
pub fn resource(
    url: impl Into<String>,
    header_mime: Option<&str>,
    size: Option<u64>,
    sha256: Option<String>,
    source_page: impl Into<String>,
    source: DiscoverySource,
    depth: u32,
) -> MediaResource {
    let url = url.into();
    let ext = extension_of(&url);
    let (mime, guessed) = match header_mime.map(|h| h.split(';').next().unwrap_or("").trim()) {
        Some(m) if !m.is_empty() => (m.to_ascii_lowercase(), false),
        _ => (
            ext.as_deref()
                .and_then(mime_from_extension)
                .unwrap_or("application/octet-stream")
                .to_string(),
            true,
        ),
    };
    let kind = resource_kind_for(&mime, ext.as_deref());
    MediaResource {
        url,
        mime,
        ext,
        size,
        sha256,
        source_page: source_page.into(),
        kind,
        source,
        depth,
        mime_guessed: guessed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> MediaPolicy {
        MediaPolicy {
            max_bytes: 1000,
            max_resources: 3,
            max_per_page: 2,
            allow: Vec::new(),
            deny: Vec::new(),
            allow_unknown_size: false,
        }
    }

    #[test]
    fn refuses_oversized_and_unknown_sizes() {
        let mut c = MediaCollector::new(policy());
        let big = resource(
            "https://x.test/a.pdf",
            Some("application/pdf"),
            Some(5000),
            None,
            "p",
            DiscoverySource::Resource,
            0,
        );
        assert_eq!(c.push(big), PushOutcome::TooLarge);
        let unknown = resource(
            "https://x.test/a.pdf",
            Some("application/pdf"),
            None,
            None,
            "p",
            DiscoverySource::Resource,
            0,
        );
        assert_eq!(c.push(unknown), PushOutcome::TooLarge);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn deny_list_wins_over_allow() {
        let mut p = policy();
        p.allow = vec!["pdf".to_string()];
        p.deny = vec!["pdf".to_string()];
        let mut c = MediaCollector::new(p);
        let r = resource(
            "https://x.test/a.pdf",
            Some("application/pdf"),
            Some(10),
            None,
            "p",
            DiscoverySource::Resource,
            0,
        );
        assert_eq!(c.push(r), PushOutcome::NotAllowed);
    }

    #[test]
    fn per_page_and_global_caps_hold() {
        let mut c = MediaCollector::new(policy());
        for i in 0..4 {
            let r = resource(
                format!("https://x.test/{i}.png"),
                Some("image/png"),
                Some(10),
                None,
                "page-a",
                DiscoverySource::Media,
                0,
            );
            c.push(r);
        }
        assert_eq!(c.len(), 2, "max_per_page=2 must hold");
        let mut c2 = MediaCollector::new(MediaPolicy {
            max_per_page: 10,
            ..policy()
        });
        for i in 0..4 {
            let r = resource(
                format!("https://x.test/{i}.png"),
                Some("image/png"),
                Some(10),
                None,
                "page-a",
                DiscoverySource::Media,
                0,
            );
            c2.push(r);
        }
        assert_eq!(c2.len(), 3, "max_resources=3 must hold");
    }

    #[test]
    fn mime_beats_extension_and_guessing_is_flagged() {
        let headered = resource(
            "https://x.test/a.bin",
            Some("image/png"),
            Some(1),
            None,
            "p",
            DiscoverySource::Media,
            0,
        );
        assert_eq!(headered.mime, "image/png");
        assert!(!headered.mime_guessed);
        let guessed = resource(
            "https://x.test/a.pdf",
            None,
            Some(1),
            None,
            "p",
            DiscoverySource::Resource,
            0,
        );
        assert_eq!(guessed.mime, "application/pdf");
        assert!(guessed.mime_guessed);
        assert_eq!(guessed.kind, ResourceKind::Document);
    }

    #[test]
    fn zero_budget_disables_collection() {
        let mut c = MediaCollector::new(MediaPolicy {
            max_bytes: 0,
            ..policy()
        });
        let r = resource(
            "https://x.test/a.pdf",
            Some("application/pdf"),
            Some(1),
            None,
            "p",
            DiscoverySource::Resource,
            0,
        );
        assert_eq!(c.push(r), PushOutcome::NotAllowed);
    }

    #[test]
    fn extension_helpers_are_bounded() {
        assert_eq!(
            extension_of("https://x.test/a/b.PDF?x=1#f").as_deref(),
            Some("pdf")
        );
        assert_eq!(extension_of("https://x.test/a/b"), None);
        assert_eq!(
            crate::content::extension_for_mime("application/pdf"),
            Some("pdf")
        );
    }
}
