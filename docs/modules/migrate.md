# Module `migrate`

Source: [`src/migrate.rs`](../../src/migrate.rs) · unit tests: [`src/migrate/tests.rs`](../../src/migrate/tests.rs)

Public surface (7 items):

- `const CURRENT_VERSION` — The schema version this build writes and understands natively.
  <!-- pub const CURRENT_VERSION: u32 = 2 -->
- `fn current_version_default` — Serde default hook so an unstamped in-memory document still round-trips as current (files on disk go through [`migrate`], which stamps v1 → v2 with a warning; a struct literal is already v2 by construction).
  <!-- pub fn current_version_default() -> u32 -->
- `struct Migration` — A single schema step. Versions must chain without gaps: v1→v2→…→ `CURRENT_VERSION`; each step takes the raw JSON value and returns transformed JSON plus user-facing notes (what changed and why).
  <!-- pub struct Migration -->
- `const MIGRATIONS` — The ordered table. Adding a breaking config change later means adding a row here (plus a test) — and nothing else; loaders, `harvest validate`, and `harvest config migrate` pick it up automatically.
  <!-- pub const MIGRATIONS: &[Migration] = &[Migration -->
- `fn check_unknown_keys` — Reject unknown top-level keys with the closest known spelling. Runs on the RAW document before migration: a typo'd key would otherwise be silently dropped by serde's lenient path, which is exactly the "silently discard" failure mode migrations must never have.
  <!-- pub fn check_unknown_keys(doc: &Value, known: &[&str]) -> Result<(), String> -->
- `fn migrate` — Migrate a raw config document up to [`CURRENT_VERSION`]. `Ok((doc, warnings))` when the document is loadable by this build; `Err` with an actionable message when it is not (future stamp, version gap, unknown key inside a migration target).
  <!-- pub fn migrate(doc: &Value) -> Result<(Value, Vec<String>), String> -->
- `fn rewritten_file` — Convenience for `harvest config migrate --out`: canonical pretty JSON with the version stamp (the migrate engine already front-places it).
  <!-- pub fn rewritten_file(doc: &Value) -> String -->
