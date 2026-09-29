# Module `runtime`

Source: [`src/runtime.rs`](../../src/runtime.rs) · unit tests: [`src/runtime/tests.rs`](../../src/runtime/tests.rs)

Public surface (5 items):

- `const BUS_CAP` — Bus ring capacity: 8 192 events is ~1-2 MB worst case of JSON values — generous for post-mortem tails, negligible for a crawl process.
  <!-- pub const BUS_CAP: usize = 8192 -->
- `struct Runtime`
  <!-- pub struct Runtime -->
- `async fn events_log_writer`
  <!-- pub async fn events_log_writer -->
- `enum LifecycleState`
  <!-- pub enum LifecycleState -->
- `struct RuntimeSlot`
  <!-- pub struct RuntimeSlot -->
