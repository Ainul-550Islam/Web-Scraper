# Module `testutil`

Source: [`src/testutil.rs`](../../src/testutil.rs) · unit tests: [`src/testutil/tests.rs`](../../src/testutil/tests.rs)

Public surface (11 items):

- `struct TestClock`
  <!-- pub struct TestClock -->
- `struct TempFixture` — A temp directory holding a written config plus its usual lanes (out/, history). Drop removes everything.
  <!-- pub struct TempFixture -->
- `fn fixture_config` — The canonical fixture config: one product-like target, generous limits, robots off (the mock serves no robots), everything under `dir`.
  <!-- pub fn fixture_config(name: &str, base_url: &str, dir: &Path) -> Value -->
- `enum Route`
  <!-- pub enum Route -->
- `struct MockServer` — Single-threaded mock HTTP/1.1 origin for tests. Longest-prefix route match; unmatched paths answer a plain 404. Every request path is recorded (`hits()`), so assertions observe what the engine actually did.
  <!-- pub struct MockServer -->
- `fn fanout_page` — A page that fans out into N `/product/<i>` links — the shape every engine-layer test has always crawled.
  <!-- pub fn fanout_page(n: usize) -> String -->
- `fn product_page`
  <!-- pub fn product_page(i: usize) -> String -->
- `struct InMemorySink`
  <!-- pub struct InMemorySink -->
- `fn collect_events` — Read an events JSONL file into values (missing file => empty).
  <!-- pub fn collect_events(path: &Path) -> Vec<Value> -->
- `fn wait_for` — Wait until a predicate holds or the deadline passes — the only allowed sleep in tests (no fixed `sleep` races).
  <!-- pub fn wait_for(deadline: Duration, mut pred: impl FnMut() -> bool) -> bool -->
- `fn metrics_view` — Snapshot metrics as JSON with derived means attached (the arithmetic the ops dashboards do; kept here so tests assert semantics, not formulas).
  <!-- pub fn metrics_view(m: &crate::metrics::Metrics) -> Value -->
