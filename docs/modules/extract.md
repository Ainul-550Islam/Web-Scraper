# Module `extract`

Source: [`src/extract.rs`](../../src/extract.rs) · unit tests: [`src/extract/tests.rs`](../../src/extract/tests.rs)

Public surface (9 items):

- `struct Extractor`
  <!-- pub struct Extractor -->
- `fn collapse_ws` — Collapse all Unicode whitespace (incl. NBSP) runs into single spaces.
  <!-- pub fn collapse_ws(s: &str) -> String -->
- `fn fingerprint` — Normalize visible text (collapse whitespace) then hash. Two identical product pages served from different URLs dedup to one fingerprint.
  <!-- pub fn fingerprint(html: &str) -> String -->
- `fn links` — Discover same-host links for crawling.
  <!-- pub fn links(doc: &Html, selector: &Selector) -> Vec<String> -->
- `enum PathStep`
  <!-- pub enum PathStep -->
- `fn compile_path` — Parse and validate a path; used by `Config::validate` so bad selectors fail before any network request is made.
  <!-- pub fn compile_path(raw: &str) -> anyhow::Result<Vec<PathStep>> -->
- `fn query_path` — Evaluate compiled steps; returns every match (wildcards expand).
  <!-- pub fn query_path<'a>(root: &'a Value, steps: &[PathStep]) -> Vec<&'a Value> -->
- `struct JsonField`
  <!-- pub struct JsonField -->
- `struct JsonExtractor`
  <!-- pub struct JsonExtractor -->
