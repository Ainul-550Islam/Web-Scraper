//! Runtime supervision: worker registration, health state, restart policy,
//! bounded background tasks, drain mode, cancellation and failure
//! propagation — one owner for the "who is alive and who died" questions
//! the engine previously answered ad hoc.
//!
//! The engine's worker pool is *N* tokio tasks pulling from a shared queue.
//! Supervisor semantics chosen for a data engine (not a service mesh):
//! * A panicking worker is reported exactly as before by default
//!   (`max_restarts = 0` → first death ends the run with the recorded
//!   error). Operators can allow bounded restarts; each restart is
//!   counted, backed off, and capped.
//! * Background tasks (checkpoint ticker, event writer) go through
//!   [`TaskSet`] so the run owns a bounded, joinable, named set — no
//!   orphaned loops survive the return.
//! * Cancellation is a token shared with the CLI's ctrl-c flag, so drain
//!   is observable from every task without new channel plumbing.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::task::JoinHandle;

/// Cooperative cancellation. Wraps the same `AtomicBool` the CLI sets on
/// ctrl-c so supervisor and signal handling can never disagree.
#[derive(Clone)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Adopt an existing shared flag (engine `stop` parameter).
    pub fn from_shared(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }
}
impl Default for CancelToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkerState {
    Idle,
    Running,
    /// Draining: finishes the current job, takes no new one.
    Draining,
    /// Exited by panic (may be restarting under policy).
    Failed,
    /// Exited normally.
    Done,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerInfo {
    pub id: usize,
    pub state: WorkerState,
    pub jobs_done: u64,
    pub jobs_failed: u64,
    pub restarts: u32,
    /// Seconds since the last recorded activity (state flip or job).
    pub idle_secs: u64,
}

#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// Extra spawn attempts after a task dies before it stays dead.
    pub max_restarts: u32,
    pub backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 0,
            backoff: Duration::from_millis(250),
        }
    }
}

#[derive(Debug)]
struct Slot {
    state: AtomicU64, // WorkerState encoded 0..=4 (see code below)
    jobs_done: AtomicU64,
    jobs_failed: AtomicU64,
    restarts: AtomicUsize,
    last_activity: Mutex<Instant>,
}

fn encode_state(s: WorkerState) -> u64 {
    match s {
        WorkerState::Idle => 0,
        WorkerState::Running => 1,
        WorkerState::Draining => 2,
        WorkerState::Failed => 3,
        WorkerState::Done => 4,
    }
}
fn decode_state(v: u64) -> WorkerState {
    match v {
        0 => WorkerState::Idle,
        1 => WorkerState::Running,
        2 => WorkerState::Draining,
        3 => WorkerState::Failed,
        _ => WorkerState::Done,
    }
}

/// Registry + policy holder, cheap to share, snapshot-able by admin.
pub struct Supervisor {
    slots: Mutex<Vec<Arc<Slot>>>,
    policy: RestartPolicy,
    cancel: CancelToken,
    total_restarts: AtomicU64,
    deaths: AtomicU64,
}

impl Supervisor {
    pub fn new(policy: RestartPolicy, cancel: CancelToken) -> Arc<Self> {
        Arc::new(Self {
            slots: Mutex::new(Vec::new()),
            policy,
            cancel,
            total_restarts: AtomicU64::new(0),
            deaths: AtomicU64::new(0),
        })
    }

    /// Register (or re-register on restart) a worker slot.
    fn register(&self, id: usize) -> Arc<Slot> {
        let mut g = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = g.get(id) {
            existing.restarts.fetch_add(1, Ordering::SeqCst);
            *existing
                .last_activity
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Instant::now();
            existing
                .state
                .store(encode_state(WorkerState::Running), Ordering::Relaxed);
            self.total_restarts.fetch_add(1, Ordering::Relaxed);
            return existing.clone();
        }
        let slot = Arc::new(Slot {
            state: AtomicU64::new(encode_state(WorkerState::Running)),
            jobs_done: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            restarts: AtomicUsize::new(0),
            last_activity: Mutex::new(Instant::now()),
        });
        g.push(slot.clone());
        slot
    }

    fn touch(&self, id: usize, state: WorkerState) {
        let g = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get(id) {
            s.state.store(encode_state(state), Ordering::Relaxed);
            *s.last_activity.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        }
    }

    /// Called by the worker task itself around each unit of work (state
    /// machine bookkeeping only; a lost update here is cosmetic).
    pub fn note_job_done(&self, id: usize) {
        let g = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get(id) {
            s.jobs_done.fetch_add(1, Ordering::Relaxed);
            *s.last_activity.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        }
    }
    pub fn note_job_failed(&self, id: usize) {
        let g = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get(id) {
            s.jobs_failed.fetch_add(1, Ordering::Relaxed);
            *s.last_activity.lock().unwrap_or_else(|p| p.into_inner()) = Instant::now();
        }
    }

    /// A worker picked up work: flip back to Running (from Idle) without
    /// touching the restart counters.
    pub fn touch_running(&self, id: usize) {
        self.touch(id, WorkerState::Running);
    }

    pub fn note_idle(&self, id: usize) {
        self.touch(id, WorkerState::Idle);
    }
    pub fn note_draining(&self, id: usize) {
        self.touch(id, WorkerState::Draining);
    }

    pub fn cancel(&self) -> &CancelToken {
        &self.cancel
    }

    /// Enter drain mode globally (shutdown ordering: stop intake, let work
    /// finish, never strand a mid-request task).
    pub fn begin_drain(&self) {
        self.cancel.cancel();
    }

    pub fn is_draining(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Run `f` as a supervised task: on JoinError (panic) restart up to
    /// policy with backoff; report the terminal death to the caller. The
    /// message format is the engine's existing contract (`worker task
    /// died:`), so default-policy supervision is a drop-in.
    pub async fn supervise<F, Fut>(self: &Arc<Self>, id: usize, f: F) -> Result<(), String>
    where
        F: Fn() -> Fut + Send + Sync,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut attempts_left = self.policy.max_restarts;
        let _slot = self.register(id);
        loop {
            let fut = f();
            let handle: JoinHandle<()> = tokio::spawn(fut);
            match handle.await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    self.deaths.fetch_add(1, Ordering::Relaxed);
                    self.touch(id, WorkerState::Failed);
                    let reported = if e.is_cancelled() {
                        format!("worker task died: cancelled (id {id})")
                    } else {
                        format!("worker task died: {e}")
                    };
                    if e.is_panic() && attempts_left > 0 && !self.is_draining() {
                        attempts_left -= 1;
                        tracing::warn!(
                            worker = id,
                            restarts_left = attempts_left,
                            "worker panicked; restarting under policy"
                        );
                        tokio::time::sleep(self.policy.backoff).await;
                        self.register(id); // counts the restart on the slot
                        continue;
                    }
                    return Err(reported);
                }
            }
        }
    }

    pub fn snapshot(&self) -> SupervisorSnapshot {
        let g = self.slots.lock().unwrap_or_else(|p| p.into_inner());
        let workers: Vec<WorkerInfo> = g
            .iter()
            .enumerate()
            .map(|(i, s)| WorkerInfo {
                id: i,
                state: decode_state(s.state.load(Ordering::Relaxed)),
                jobs_done: s.jobs_done.load(Ordering::Relaxed),
                jobs_failed: s.jobs_failed.load(Ordering::Relaxed),
                restarts: s.restarts.load(Ordering::SeqCst) as u32,
                idle_secs: s
                    .last_activity
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .elapsed()
                    .as_secs(),
            })
            .collect();
        let busy = workers
            .iter()
            .filter(|w| w.state == WorkerState::Running)
            .count();
        SupervisorSnapshot {
            registered: workers.len(),
            busy,
            utilization: if workers.is_empty() {
                0.0
            } else {
                busy as f64 / workers.len() as f64
            },
            total_restarts: self.total_restarts.load(Ordering::Relaxed),
            deaths: self.deaths.load(Ordering::Relaxed),
            draining: self.is_draining(),
            workers,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SupervisorSnapshot {
    pub registered: usize,
    pub busy: usize,
    pub utilization: f64,
    pub total_restarts: u64,
    pub deaths: u64,
    pub draining: bool,
    pub workers: Vec<WorkerInfo>,
}

/// Bounded, named background tasks that MUST not outlive the run. Drop
/// without `shutdown` aborts everything (the safety net); `shutdown` joins
/// with a deadline, then aborts what remains.
pub struct TaskSet {
    cap: usize,
    tasks: Mutex<BTreeMap<&'static str, JoinHandle<()>>>,
}

impl TaskSet {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: cap.max(1),
            tasks: Mutex::new(BTreeMap::new()),
        }
    }

    /// Register a named task; replacing a name aborts the old handle
    /// (restart semantics), exceeding the cap refuses (returns false) —
    /// a bounded set cannot be talked into unbounded growth.
    pub fn spawn(
        &self,
        name: &'static str,
        fut: impl std::future::Future<Output = ()> + Send + 'static,
    ) -> bool {
        let mut g = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        if !g.contains_key(name) && g.len() >= self.cap {
            return false;
        }
        if let Some(old) = g.insert(name, tokio::spawn(fut)) {
            old.abort();
        }
        true
    }

    pub fn len(&self) -> usize {
        self.tasks.lock().unwrap_or_else(|p| p.into_inner()).len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Join everything (each with `per_task` allowance), then abort strays.
    pub async fn shutdown(&self, per_task: Duration) -> Vec<String> {
        let drained: BTreeMap<_, _> = {
            let mut g = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *g)
        };
        let mut stragglers = Vec::new();
        for (name, h) in drained {
            tokio::pin!(h);
            if tokio::time::timeout(per_task, &mut h).await.is_err() {
                h.abort(); // no orphans: a straggler is killed, then reported
                stragglers.push(name.to_string());
            }
        }
        stragglers
    }
}

impl Drop for TaskSet {
    fn drop(&mut self) {
        let g = std::mem::take(&mut *self.tasks.lock().unwrap_or_else(|p| p.into_inner()));
        for (_, h) in g {
            h.abort();
        }
    }
}

#[cfg(test)]
mod tests;
