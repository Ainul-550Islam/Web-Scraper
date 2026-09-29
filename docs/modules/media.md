# Module `media`

Source: [`src/media.rs`](../../src/media.rs)

Public surface (8 items):

- `enum ResourceKind`
  <!-- pub enum ResourceKind -->
- `struct MediaResource`
  <!-- pub struct MediaResource -->
- `enum PushOutcome`
  <!-- pub enum PushOutcome -->
- `struct MediaPolicy`
  <!-- pub struct MediaPolicy -->
- `struct MediaCollector`
  <!-- pub struct MediaCollector -->
- `fn extension_of` — Lowercased file extension of a URL path, when it has one. Reported for the
  <!-- pub fn extension_of(url: &str) -> Option<String> -->
- `fn resource_kind_for` — Classify a resource by MIME first, extension second.
  <!-- pub fn resource_kind_for(mime: &str, ext: Option<&str>) -> ResourceKind -->
- `fn resource`
  <!-- pub fn resource( -->
