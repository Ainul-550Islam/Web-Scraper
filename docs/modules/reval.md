# Module `reval`

Source: [`src/reval.rs`](../../src/reval.rs) · unit tests: [`src/reval/tests.rs`](../../src/reval/tests.rs)

Public surface (4 items):

- `const LINK_CAP` — Cap the cached link set per document: nav-heavy pages (thousands of hrefs) must not turn the store into a memory hog. Above the cap, a 304 replays only the first `LINK_CAP` links — acceptable, documented.
  <!-- pub const LINK_CAP: usize = 500 -->
- `struct UrlState`
  <!-- pub struct UrlState -->
- `enum Condition`
  <!-- pub enum Condition -->
- `struct Store`
  <!-- pub struct Store -->
