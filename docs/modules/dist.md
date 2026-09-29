# Module `dist`

Source: [`src/dist.rs`](../../src/dist.rs) · unit tests: [`src/dist/tests.rs`](../../src/dist/tests.rs)

Public surface (6 items):

- `struct Job`
  <!-- pub struct Job -->
- `struct Queue`
  <!-- pub struct Queue -->
- `struct Sweep`
  <!-- pub struct Sweep -->
- `const DEFAULT_LEASE_TTL_SECS` — Default lease heartbeat TTL. A killed worker's leases expire after this and its strays become reclaimable; a live worker refreshes at TTL/3.
  <!-- pub const DEFAULT_LEASE_TTL_SECS: u64 = 30 -->
- `struct WorkerStats`
  <!-- pub struct WorkerStats -->
- `struct WorkerOpts`
  <!-- pub struct WorkerOpts -->
