# Module `download`

Source: [`src/download.rs`](../../src/download.rs)

Unit tests: [`src/download/tests.rs`](../../src/download/tests.rs)

Public surface (12 items):

**src/download.rs**

- `const MAX_ARTIFACTS` — How many artifacts the registry retains. Oldest finalized entries are
  <!-- pub const MAX_ARTIFACTS: usize = 256 -->
- `enum ArtifactState`
  <!-- pub enum ArtifactState -->
- `enum ArtifactRole`
  <!-- pub enum ArtifactRole -->
- `enum VerificationState`
  <!-- pub enum VerificationState -->
- `struct ArtifactRecord`
  <!-- pub struct ArtifactRecord -->
- `fn content_type_for` — MIME type for a registered file name. Unknown extensions get
  <!-- pub fn content_type_for(name: &str) -> &'static str -->
- `fn safe_filename` — Strip anything that could break out of a `Content-Disposition` header:
  <!-- pub fn safe_filename(name: &str) -> String -->
- `fn content_disposition` — `attachment; filename="…"` — the only form we emit, so a download never
  <!-- pub fn content_disposition(name: &str) -> String -->
- `enum DownloadErrorKind`
  <!-- pub enum DownloadErrorKind -->
- `struct DownloadError`
  <!-- pub struct DownloadError -->
- `struct OpenedArtifact`
  <!-- pub struct OpenedArtifact -->
- `struct ArtifactStore` — Bounded registry of downloadable artifacts.
  <!-- pub struct ArtifactStore -->
