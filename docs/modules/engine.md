# Module `engine`

Source: [`src/engine.rs`](../../src/engine.rs) · unit tests: [`src/engine/tests.rs`](../../src/engine/tests.rs)

Public surface (4 items):

- `struct TargetSummary`
  <!-- pub struct TargetSummary -->
- `struct Summary`
  <!-- pub struct Summary -->
- `struct RunCtx` — Per-run shared context usable by the admin dashboard. `runtime`: slot filled by the engine when a run starts; the admin `/runtime` and `/events` routes read through it. `Default` for embedders that only care about metrics/proxies.
  <!-- pub struct RunCtx -->
- `fn ua_token`
  <!-- pub fn ua_token(ua: &str) -> String -->
