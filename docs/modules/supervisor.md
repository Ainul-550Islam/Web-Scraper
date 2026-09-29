# Module `supervisor`

Source: [`src/supervisor.rs`](../../src/supervisor.rs) · unit tests: [`src/supervisor/tests.rs`](../../src/supervisor/tests.rs)

Public surface (7 items):

- `struct CancelToken`
  <!-- pub struct CancelToken -->
- `enum WorkerState`
  <!-- pub enum WorkerState -->
- `struct WorkerInfo`
  <!-- pub struct WorkerInfo -->
- `struct RestartPolicy`
  <!-- pub struct RestartPolicy -->
- `struct Supervisor` — Registry + policy holder, cheap to share, snapshot-able by admin.
  <!-- pub struct Supervisor -->
- `struct SupervisorSnapshot`
  <!-- pub struct SupervisorSnapshot -->
- `struct TaskSet` — Bounded, named background tasks that MUST not outlive the run. Drop without `shutdown` aborts everything (the safety net); `shutdown` joins with a deadline, then aborts what remains.
  <!-- pub struct TaskSet -->
