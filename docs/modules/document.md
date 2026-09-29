# Module `document`

Source: [`src/document.rs`](../../src/document.rs)

Public surface (8 items):

- `enum DiscoverySource`
  <!-- pub enum DiscoverySource -->
- `struct Document`
  <!-- pub struct Document -->
- `enum PushOutcome`
  <!-- pub enum PushOutcome -->
- `struct DocumentSet`
  <!-- pub struct DocumentSet -->
- `fn canonical_url_key` — Cheap URL key for de-duplication: lowercase scheme+host, drop the fragment
  <!-- pub fn canonical_url_key(url: &str) -> String -->
- `fn title_from_html` — Extract `<title>` text from HTML without pulling in a parser pass: the
  <!-- pub fn title_from_html(html: &str) -> Option<String> -->
- `fn sha256_hex` — sha256 hex of a body. Shared by the sink, export and document paths so a
  <!-- pub fn sha256_hex(bytes: &[u8]) -> String -->
- `fn safe_dataset_name` — Deterministic, filesystem-safe dataset id: `dataset-<id>.<ext>`. Anything
  <!-- pub fn safe_dataset_name(id: &str) -> String -->
