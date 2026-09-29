# Module `execution`

Source: [`src/execution.rs`](../../src/execution.rs)

Unit tests: [`src/execution/tests.rs`](../../src/execution/tests.rs)

Public surface (10 items):

**src/execution.rs**

- `const MAX_JOBS` — Retained job records (terminal ones are evicted oldest-first).
  <!-- pub const MAX_JOBS: usize = 256 -->
- `enum ExecState`
  <!-- pub enum ExecState -->
- `struct JobCounters`
  <!-- pub struct JobCounters -->
- `struct JobEvent`
  <!-- pub struct JobEvent -->
- `const MAX_EVENTS` — Retained events per job.
  <!-- pub const MAX_EVENTS: usize = 32 -->
- `struct JobSnapshot`
  <!-- pub struct JobSnapshot -->
- `struct JobFilter`
  <!-- pub struct JobFilter -->
- `struct JobFailure`
  <!-- pub struct JobFailure -->
- `struct SubmitOutcome`
  <!-- pub struct SubmitOutcome -->
- `struct ExecutionRegistry` — The coordinator. Cheap to clone into spawned tasks (`Arc`).
  <!-- pub struct ExecutionRegistry -->
