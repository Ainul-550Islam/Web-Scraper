# Module `fetcher`

Source: [`src/fetcher.rs`](../../src/fetcher.rs) · unit tests: [`src/fetcher/tests.rs`](../../src/fetcher/tests.rs)

Public surface (5 items):

- `enum FetchError`
  <!-- pub enum FetchError -->
- `struct FetchOutcome`
  <!-- pub struct FetchOutcome -->
- `struct Fetcher`
  <!-- pub struct Fetcher -->
- `struct AttemptTrace`
  <!-- pub struct AttemptTrace -->
- `fn parse_retry_after` — Parse Retry-After: delay-seconds or HTTP date (best effort).
  <!-- pub fn parse_retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> -->
