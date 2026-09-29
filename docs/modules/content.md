# Module `content`

Source: [`src/content.rs`](../../src/content.rs)

Public surface (16 items):

- `const MAX_SNIFF_BYTES` — How much of a body any sniffer is allowed to look at. Bounded so a 200 MB
  <!-- pub const MAX_SNIFF_BYTES: usize = 8192; -->
- `const DEFAULT_MAX_BODY_BYTES` — Fallback body cap used when a caller has no configured limit. Callers are
  <!-- pub const DEFAULT_MAX_BODY_BYTES: u64 = 8 * 1024 * 1024; -->
- `enum ContentKind`
  <!-- pub enum ContentKind -->
- `struct TooLarge`
  <!-- pub struct TooLarge -->
- `struct ContentInfo`
  <!-- pub struct ContentInfo -->
- `fn parse_content_type` — Parse a `Content-Type` header value into (mime, charset). Only the first
  <!-- pub fn parse_content_type(raw: &str) -> (Option<String>, Option<String>) -->
- `fn normalize_mime` — Strip parameters and casing: `Text/HTML; charset=utf-8` -> `text/html`.
  <!-- pub fn normalize_mime(raw: &str) -> String -->
- `fn kind_from_mime` — Map a canonical MIME type onto a [`ContentKind`]. `None` means "recognized
  <!-- pub fn kind_from_mime(mime: &str) -> Option<ContentKind> -->
- `fn mime_from_extension` — Extension -> canonical MIME, used only to *report* a resource's type when
  <!-- pub fn mime_from_extension(ext: &str) -> Option<&'static str> -->
- `fn extension_for_mime` — Canonical MIME -> the extension used for delivered files.
  <!-- pub fn extension_for_mime(mime: &str) -> Option<&'static str> -->
- `fn is_probably_binary` — Heuristic: does this body look like binary rather than text? A NUL byte in
  <!-- pub fn is_probably_binary(body: &[u8]) -> bool -->
- `fn classify` — Classify a body from an optional `Content-Type` header plus its bytes.
  <!-- pub fn classify(header: Option<&str>, body: &[u8]) -> ContentInfo -->
- `fn classify_bounded` — Same as [`classify`], but rejects bodies over `max_bytes` first. Order
  <!-- pub fn classify_bounded( -->
- `fn sniff_charset` — Detect UTF-16/UTF-8 BOMs so callers can decode rather than mangle.
  <!-- pub fn sniff_charset(body: &[u8]) -> Option<String> -->
- `fn decode_text` — Decode a body to text using the classified charset. Total: never fails.
  <!-- pub fn decode_text(info: &ContentInfo, body: &[u8]) -> String -->
- `fn normalize_text` — Safe, deterministic text normalization: strip the BOM, normalize CRLF/CR to
  <!-- pub fn normalize_text(s: &str) -> String -->
