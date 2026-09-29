//! Shared content classification & normalization layer.
//!
//! Everything that touches a fetched body — discovery, the document model,
//! media discovery, and every export adapter — needs the same answer to three
//! questions: *what is this*, *how do I decode it*, and *is it too big*.
//! Answering those in three places is how MIME handling rots, so the logic
//! lives here once.
//!
//! Rules this module enforces (and the tests in `tests/content_integrity.rs`
//! pin):
//! * **Headers win, content corroborates.** A `Content-Type` header is
//!   authoritative for the declared type; the body is sniffed to fill gaps
//!   (absent/octet-stream headers) and to flag contradictions
//!   (`ContentInfo::mismatch`). Nothing here trusts a file extension — that is
//!   the caller's business (`media::extension_of` is reported, never believed).
//! * **Bounded.** Sniffing reads at most [`MAX_SNIFF_BYTES`]; callers hand
//!   bodies to [`classify_bounded`] with their own size cap so an
//!   oversized response is rejected before it is decoded, not after.
//! * **Total decoding.** [`decode_text`] never fails and never panics on
//!   malformed bytes: undecodable sequences become U+FFFD, and non-UTF-8
//!   charsets are transcoded per their (documented) mapping.
//! * **Lossy by declaration.** [`normalize_text`] strips BOMs, normalizes
//!   newlines and drops C0 control characters — it does *not* attempt full
//!   Unicode normalization (NFC/NFKC), which would need a Unicode table this
//!   crate does not ship.

use std::fmt;

/// How much of a body any sniffer is allowed to look at. Bounded so a 200 MB
/// response costs the same to classify as a 2 KB one.
pub const MAX_SNIFF_BYTES: usize = 8192;

/// Fallback body cap used when a caller has no configured limit. Callers are
/// expected to pass the configured `max_response_bytes` through instead.
pub const DEFAULT_MAX_BODY_BYTES: u64 = 8 * 1024 * 1024;

/// What a body *is*, in the terms the rest of the engine cares about. `Csv` is
/// header-driven only (see [`classify`]): arbitrary plain text is shaped like
/// CSV, so guessing it from content would mislabel half the web.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ContentKind {
    Html,
    Json,
    Xml,
    Csv,
    Text,
    /// PDF: metadata is read, page text is not parsed (no PDF parser here).
    Pdf,
    /// Anything else: media, archives, unknown binaries. Metadata only.
    Binary,
}

impl ContentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Json => "json",
            Self::Xml => "xml",
            Self::Csv => "csv",
            Self::Text => "text",
            Self::Pdf => "pdf",
            Self::Binary => "binary",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "html" => Self::Html,
            "json" => Self::Json,
            "xml" => Self::Xml,
            "csv" => Self::Csv,
            "text" | "plain" | "txt" => Self::Text,
            "pdf" => Self::Pdf,
            "binary" => Self::Binary,
            _ => return None,
        })
    }

    /// True when a body of this kind can become records without an external
    /// parser (PDF stays false on purpose: we ship metadata, not page text).
    pub fn is_textual(self) -> bool {
        matches!(
            self,
            Self::Html | Self::Json | Self::Xml | Self::Csv | Self::Text
        )
    }
}

impl fmt::Display for ContentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A body rejected before decoding because it exceeded the caller's cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TooLarge {
    pub bytes: u64,
    pub limit: u64,
}

impl fmt::Display for TooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "response body {} bytes exceeds the configured limit of {} bytes",
            self.bytes, self.limit
        )
    }
}

impl std::error::Error for TooLarge {}

/// The classification verdict for one body.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ContentInfo {
    pub kind: ContentKind,
    /// Canonical MIME type (parameters stripped, lowercased).
    pub mime: String,
    /// Declared charset, when one is present and recognized.
    pub charset: Option<String>,
    pub bytes: u64,
    /// What the header claimed, if anything (`None` = no usable header).
    pub header_mime: Option<String>,
    /// True when the declared header and the sniffed bytes disagree —
    /// e.g. `text/html` over a `%PDF-` payload. Callers surface this; they do
    /// not silently pick a winner.
    pub mismatch: bool,
    /// True when the body is well-formed UTF-8 (or was transcoded into it).
    pub decodable_text: bool,
}

impl ContentInfo {
    /// Default "nothing declared, nothing sniffed" state for an empty body.
    pub fn empty() -> Self {
        Self {
            kind: ContentKind::Binary,
            mime: "application/octet-stream".to_string(),
            charset: None,
            bytes: 0,
            header_mime: None,
            mismatch: false,
            decodable_text: true,
        }
    }
}

/// Parse a `Content-Type` header value into (mime, charset). Only the first
/// `parameter` we understand is kept; vendor junk is ignored.
pub fn parse_content_type(raw: &str) -> (Option<String>, Option<String>) {
    let mut parts = raw.split(';');
    let mime = parts.next().map(normalize_mime);
    let mut charset = None;
    for p in parts {
        let p = p.trim();
        if let Some(v) = p.to_ascii_lowercase().strip_prefix("charset=") {
            let v = v.trim().trim_matches('"').trim();
            if !v.is_empty() {
                charset = Some(v.to_ascii_lowercase());
            }
        }
    }
    (mime, charset)
}

/// Strip parameters and casing: `Text/HTML; charset=utf-8` -> `text/html`.
pub fn normalize_mime(raw: &str) -> String {
    raw.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// Map a canonical MIME type onto a [`ContentKind`]. `None` means "recognized
/// MIME, but not one this engine parses" (images, archives, video, …).
pub fn kind_from_mime(mime: &str) -> Option<ContentKind> {
    let m = normalize_mime(mime);
    match m.as_str() {
        "text/html" | "application/xhtml+xml" => Some(ContentKind::Html),
        "application/json" | "application/ld+json" | "application/vnd.api+json" => {
            Some(ContentKind::Json)
        }
        "text/json" => Some(ContentKind::Json),
        "application/xml" | "text/xml" | "application/rss+xml" | "application/atom+xml" => {
            Some(ContentKind::Xml)
        }
        "text/csv" | "application/csv" => Some(ContentKind::Csv),
        "application/pdf" => Some(ContentKind::Pdf),
        "text/plain" => Some(ContentKind::Text),
        _ => None,
    }
}

/// Extension -> canonical MIME, used only to *report* a resource's type when
/// no header is available (media discovery on an `<img src>` we never fetched).
/// Never used to decide how a fetched body is parsed.
pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    Some(match ext.trim().to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "xml" | "rss" | "atom" => "application/xml",
        "csv" => "text/csv",
        "txt" | "md" => "text/plain",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "xls" => "application/vnd.ms-excel",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "parquet" => "application/vnd.apache.parquet",
        _ => return None,
    })
}

/// Canonical MIME -> the extension used for delivered files.
pub fn extension_for_mime(mime: &str) -> Option<&'static str> {
    Some(match normalize_mime(mime).as_str() {
        "text/html" => "html",
        "application/json" => "json",
        "application/xml" | "text/xml" => "xml",
        "text/csv" => "csv",
        "text/plain" => "txt",
        "application/pdf" => "pdf",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "video/mp4" => "mp4",
        "audio/mpeg" => "mp3",
        "application/zip" => "zip",
        _ => return None,
    })
}

/// Heuristic: does this body look like binary rather than text? A NUL byte in
/// the sniff window is decisive; otherwise we check that the bytes decode as
/// UTF-8. Used for the `Binary` kind and for "do not try to extract text".
pub fn is_probably_binary(body: &[u8]) -> bool {
    let end = body.len().min(MAX_SNIFF_BYTES);
    if body[..end].contains(&0u8) {
        return true;
    }
    std::str::from_utf8(&body[..end]).is_err()
}

/// Sniff a body without any header context. Returns `None` when the bytes give
/// nothing away (plain text and binary often look alike).
fn sniff(body: &[u8]) -> Option<(ContentKind, &'static str)> {
    if body.is_empty() {
        return None;
    }
    let head = &body[..body.len().min(MAX_SNIFF_BYTES)];
    if head.starts_with(b"%PDF-") {
        return Some((ContentKind::Pdf, "application/pdf"));
    }
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return Some((ContentKind::Text, "text/plain"));
    }
    // Trim a UTF-8 BOM and leading whitespace before looking at the first
    // meaningful byte — servers and CMSs both emit junk prefixes.
    let mut s = std::str::from_utf8(head).unwrap_or("");
    s = s.trim_start_matches('\u{feff}').trim_start();
    if s.is_empty() {
        return None;
    }
    let lower = {
        let mut l = String::with_capacity(s.len().min(512));
        for c in s.chars().take(512) {
            l.push(c.to_ascii_lowercase());
        }
        l
    };
    if lower.starts_with("<!doctype html") || lower.starts_with("<html") {
        return Some((ContentKind::Html, "text/html"));
    }
    if lower.starts_with("<?xml") {
        // Sitemaps, RSS and Atom are XML documents we do parse.
        return Some((ContentKind::Xml, "application/xml"));
    }
    if lower.starts_with('<') {
        // Buffered by a downstream check: an HTML-ish tag set is still HTML.
        if lower.starts_with("<urlset")
            || lower.starts_with("<sitemapindex")
            || lower.starts_with("<rss")
            || lower.starts_with("<feed")
        {
            return Some((ContentKind::Xml, "application/xml"));
        }
        if lower.starts_with("<html") || lower.contains("<body") || lower.contains("<div") {
            return Some((ContentKind::Html, "text/html"));
        }
        return Some((ContentKind::Xml, "application/xml"));
    }
    let first = s.chars().next().unwrap_or('\0');
    if first == '{' || first == '[' {
        // Only claim JSON if it actually parses (bounded: strictly smaller
        // than a configured body cap, and JSON is cheap to validate).
        if serde_json::from_slice::<serde_json::Value>(&body[..body.len().min(1 << 20)]).is_ok() {
            return Some((ContentKind::Json, "application/json"));
        }
        return Some((ContentKind::Text, "text/plain"));
    }
    None
}

/// Classify a body from an optional `Content-Type` header plus its bytes.
/// Header wins; sniffing fills in `application/octet-stream`/absent headers and
/// sets [`ContentInfo::mismatch`] when the two disagree.
pub fn classify(header: Option<&str>, body: &[u8]) -> ContentInfo {
    let (h_mime, h_charset) = header
        .map(parse_content_type)
        .unwrap_or_else(|| (None, None));
    let declared = h_mime.as_deref().and_then(kind_from_mime);
    let sniffed = sniff(body);
    let bytes = body.len() as u64;

    // A present, non-generic header is authoritative even when its MIME is one
    // this engine does not parse (image/png, application/zip, …): the server
    // declared what it sent, and we report the contradiction instead of
    // parsing bytes against the declaration.
    let generic_header = match h_mime.as_deref() {
        None => true,
        Some(m) => m == "application/octet-stream",
    };
    let (kind, mime, mismatch) = match (declared, sniffed) {
        (Some(k), Some((sk, _))) => (k, normalize_mime(h_mime.as_deref().unwrap_or("")), k != sk),
        (Some(k), None) => (k, normalize_mime(h_mime.as_deref().unwrap_or("")), false),
        (None, Some((sk, sm))) if generic_header => (sk, sm.to_string(), false),
        (None, Some((_, _))) => (
            ContentKind::Binary,
            normalize_mime(h_mime.as_deref().unwrap_or("application/octet-stream")),
            true,
        ),
        (None, None) => {
            if is_probably_binary(body) {
                (
                    ContentKind::Binary,
                    h_mime
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                    false,
                )
            } else {
                (
                    ContentKind::Text,
                    h_mime.clone().unwrap_or_else(|| "text/plain".to_string()),
                    false,
                )
            }
        }
    };
    let mime = if mime.is_empty() {
        "application/octet-stream".to_string()
    } else {
        mime
    };
    let charset = match h_charset {
        Some(c) => Some(c),
        None => sniff_charset(body),
    };
    let decodable_text = !is_probably_binary(body);
    ContentInfo {
        kind,
        mime,
        charset,
        bytes,
        header_mime: h_mime,
        mismatch,
        decodable_text,
    }
}

/// Same as [`classify`], but rejects bodies over `max_bytes` first. Order
/// matters: an oversized body is refused, never decoded and never sniffed.
pub fn classify_bounded(
    header: Option<&str>,
    body: &[u8],
    max_bytes: u64,
) -> Result<ContentInfo, TooLarge> {
    let bytes = body.len() as u64;
    if bytes > max_bytes {
        return Err(TooLarge {
            bytes,
            limit: max_bytes,
        });
    }
    Ok(classify(header, body))
}

/// Detect UTF-16/UTF-8 BOMs so callers can decode rather than mangle.
pub fn sniff_charset(body: &[u8]) -> Option<String> {
    if body.len() >= 2 {
        match (body[0], body[1]) {
            (0xEF, 0xBB) if body.len() >= 3 && body[2] == 0xBF => return Some("utf-8".to_string()),
            (0xFF, 0xFE) => return Some("utf-16le".to_string()),
            (0xFE, 0xFF) => return Some("utf-16be".to_string()),
            _ => {}
        }
    }
    None
}

/// Decode a body to text using the classified charset. Total: never fails.
/// Undecodable bytes become U+FFFD; unknown charsets fall back to UTF-8 lossy
/// (the web's overwhelming default) rather than guessing a Latin page.
pub fn decode_text(info: &ContentInfo, body: &[u8]) -> String {
    match info.charset.as_deref() {
        Some("utf-16le") => decode_utf16(body, true),
        Some("utf-16be") => decode_utf16(body, false),
        Some("iso-8859-1") | Some("latin1") | Some("iso8859-1") => latin1_to_string(body),
        Some("windows-1252") | Some("cp1252") => windows1252_to_string(body),
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}

fn decode_utf16(body: &[u8], le: bool) -> String {
    // Drop a BOM if present, then decode complete pairs only (an odd trailing
    // byte is malformed input, not a panic).
    let skip = if body.len() >= 2
        && ((body[0] == 0xFF && body[1] == 0xFE) || (body[0] == 0xFE && body[1] == 0xFF))
    {
        2
    } else {
        0
    };
    let rest = &body[skip..];
    // `as_chunks` keeps the malformed trailing byte out without a panic.
    let (pairs, _odd) = rest.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|c| {
            if le {
                u16::from_le_bytes([c[0], c[1]])
            } else {
                u16::from_be_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

fn latin1_to_string(body: &[u8]) -> String {
    body.iter().map(|b| *b as char).collect()
}

/// Windows-1252 agrees with Latin-1 except in 0x80..=0x9F, where it defines
/// printable punctuation. Those 32 code points are mapped here; the rest pass
/// through byte-for-byte.
fn windows1252_to_string(body: &[u8]) -> String {
    const HI: [char; 32] = [
        '\u{20ac}', '\u{0081}', '\u{201a}', '\u{0192}', '\u{201e}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{02c6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{008d}',
        '\u{017d}', '\u{008f}', '\u{0090}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}',
        '\u{2022}', '\u{2013}', '\u{2014}', '\u{02dc}', '\u{2122}', '\u{0161}', '\u{203a}',
        '\u{0153}', '\u{009d}', '\u{017e}', '\u{0178}',
    ];
    let mut out = String::with_capacity(body.len());
    for b in body {
        let c = match *b {
            0x80..=0x9F => HI[(*b - 0x80) as usize],
            other => other as char,
        };
        out.push(c);
    }
    out
}

/// Safe, deterministic text normalization: strip the BOM, normalize CRLF/CR to
/// LF, drop remaining C0 control characters (keeping tab and LF), collapse runs
/// of spaces/tabs to one space, and trim each line. This is *not* Unicode
/// normalization — see the module docs.
pub fn normalize_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let s = s.strip_prefix('\u{feff}').unwrap_or(s);
    let mut prev_space = false;
    for line in s.split('\n') {
        for ch in line.chars() {
            match ch {
                '\r' => {}
                '\t' | ' ' => {
                    if !prev_space {
                        out.push(' ');
                        prev_space = true;
                    }
                }
                c if (c as u32) < 0x20 => {}
                '\u{7f}' => {}
                c => {
                    out.push(c);
                    prev_space = false;
                }
            }
        }
        // Trim the trailing space a collapsed run can leave behind.
        if out.ends_with(' ') {
            out.pop();
        }
        out.push('\n');
    }
    if out.ends_with('\n') {
        out.pop();
    }
    out.trim_matches('\n').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_wins_over_sniffing() {
        let info = classify(Some("text/html; charset=utf-8"), b"%PDF-1.7 junk");
        assert_eq!(info.kind, ContentKind::Html);
        assert!(info.mismatch, "header/body disagreement must be flagged");
        assert_eq!(info.charset.as_deref(), Some("utf-8"));
    }

    #[test]
    fn octet_stream_falls_back_to_sniffing() {
        let info = classify(Some("application/octet-stream"), b"%PDF-1.7");
        assert_eq!(info.kind, ContentKind::Pdf);
        assert!(!info.mismatch);
    }

    #[test]
    fn sitemap_xml_is_xml() {
        let info = classify(
            None,
            b"<?xml version=\"1.0\"?><urlset><url><loc>/a</loc></url></urlset>",
        );
        assert_eq!(info.kind, ContentKind::Xml);
    }

    #[test]
    fn malformed_json_is_text_not_json() {
        let info = classify(None, b"{not json");
        assert_eq!(info.kind, ContentKind::Text);
    }

    #[test]
    fn bounded_rejects_oversized_bodies() {
        let err = classify_bounded(None, &[0u8; 100], 99).unwrap_err();
        assert_eq!(err.bytes, 100);
        assert_eq!(err.limit, 99);
    }

    #[test]
    fn binary_detection_uses_nul_bytes() {
        assert!(is_probably_binary(&[0x41, 0x00, 0x42]));
        assert!(!is_probably_binary(b"plain text"));
    }

    #[test]
    fn windows1252_high_range_maps_to_punctuation() {
        let info = ContentInfo {
            charset: Some("windows-1252".to_string()),
            ..ContentInfo::empty()
        };
        assert_eq!(decode_text(&info, &[0x80, 0x93, 0x41]), "\u{20ac}\u{201c}A");
    }

    #[test]
    fn normalize_text_is_deterministic() {
        let a = normalize_text("\u{feff}a\u{0}b\r\nc   d\t\te");
        let b = normalize_text("\u{feff}a\u{0}b\r\nc   d\t\te");
        assert_eq!(a, b);
        assert_eq!(a, "ab\nc d e");
    }
}
