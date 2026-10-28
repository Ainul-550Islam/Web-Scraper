//! Scheduler: the queue the crawl workers pull from, with explicit policies
//! — FIFO (default, legacy-exact), priority, per-target fair round-robin —
//! plus bounded depth (backpressure), per-target inflight limits,
//! cancellation-aware pops, deadline enforcement and queue statistics.
//!
//! One invariant carries the engine's behavior: with the default
//! configuration (`mode = "fifo"`, no caps, no deadlines) this structure is
//! order-and-semantics identical to the raw `Mutex<VecDeque>` it replaced.
//! The features are opt-in; the abstraction is free.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::config::SchedulingCfg;
use crate::job::{Job, JobId, JobPriority, JobState};

/// Result of asking the queue for work.
#[derive(Debug)]
pub enum Popped {
    /// Ready job, already marked Running and counted against its target's
    /// inflight allowance.
    Job(Job),
    /// A queued job passed its deadline; engine notes it (event + metric)
    /// and pulls again.
    Expired(Job),
    /// Nothing to do right now.
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushError {
    /// The bounded queue is full — caller drops the job (discovery stop),
    /// counted as backpressure.
    Full,
    /// The job targets an index not present in this queue's target table.
    InvalidTarget,
}

/// The shared crawl queue.
pub struct CrawlQueue {
    mode: SchedulingMode,
    max_depth: usize,           // 0 = unbounded
    per_target_inflight: usize, // 0 = unbounded (fair mode only applies it)
    target_count: usize,
    inner: Mutex<Inner>,
    pushed: AtomicU64,
    rejected: AtomicU64,
    expired: AtomicU64,
    popped: AtomicU64,
    inflight: AtomicUsize,
    // target -> currently-executing count (shared atomics for fair gating)
    active: Mutex<Vec<AtomicUsize>>,
    rr: AtomicUsize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingMode {
    Fifo,
    Priority,
    Fair,
}

impl SchedulingMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fifo" => Some(Self::Fifo),
            "priority" => Some(Self::Priority),
            "fair" => Some(Self::Fair),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fifo => "fifo",
            Self::Priority => "priority",
            Self::Fair => "fair",
        }
    }
}

#[derive(Debug, Default)]
struct Inner {
    fifo: VecDeque<Job>,
    /// heap entries: (rank desc via reverse key below, seq asc for FIFO-in-prio)
    prio: Vec<Job>, // kept sorted on push (small queues; O(log n) via sort_unstable by key)
    per_target: Vec<VecDeque<Job>>,
}

impl CrawlQueue {
    pub fn new(cfg: &SchedulingCfg, n_targets: usize) -> Arc<Self> {
        let mode = SchedulingMode::parse(&cfg.mode).unwrap_or(SchedulingMode::Fifo);
        let mut inner = Inner::default();
        if mode == SchedulingMode::Fair {
            inner.per_target = (0..n_targets.max(1)).map(|_| VecDeque::new()).collect();
        }
        Arc::new(Self {
            mode,
            max_depth: cfg.max_queue_depth as usize,
            per_target_inflight: cfg.per_target_inflight as usize,
            target_count: n_targets,
            inner: Mutex::new(inner),
            pushed: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            expired: AtomicU64::new(0),
            popped: AtomicU64::new(0),
            inflight: AtomicUsize::new(0),
            active: Mutex::new((0..n_targets.max(1)).map(|_| AtomicUsize::new(0)).collect()),
            rr: AtomicUsize::new(0),
        })
    }

    fn depth_of(&self, g: &Inner) -> usize {
        match self.mode {
            SchedulingMode::Fifo => g.fifo.len(),
            SchedulingMode::Priority => g.prio.len(),
            SchedulingMode::Fair => g.per_target.iter().map(|q| q.len()).sum(),
        }
    }

    /// Enqueue for crawling. `Err(Full)` when the bounded depth is reached —
    /// the caller (discovery) simply stops adding, which IS the backpressure
    /// signal; nothing is evicted and nothing blocks a worker. `InvalidTarget`
    /// refuses indices outside the target table before they can reach a worker.
    pub fn push(&self, mut job: Job) -> Result<JobId, PushError> {
        job.state = JobState::Pending;
        let id = job.id;
        if job.target >= self.target_count {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(PushError::InvalidTarget);
        }
        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if self.max_depth > 0 && self.depth_of(&g) >= self.max_depth {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(PushError::Full);
            }
            Self::insert(&mut g, self.mode, job);
        }
        self.pushed.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    /// Return an already-admitted in-flight job to the pending queue. This
    /// bypasses the waiting-depth cap so shutdown/cancellation cannot lose
    /// accepted work when other workers fill the queue while it is running.
    /// The temporary excess is bounded by the number of active workers.
    pub fn requeue(&self, mut job: Job) -> Result<JobId, PushError> {
        job.state = JobState::Pending;
        let id = job.id;
        if job.target >= self.target_count {
            self.rejected.fetch_add(1, Ordering::Relaxed);
            return Err(PushError::InvalidTarget);
        }
        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            Self::insert(&mut g, self.mode, job);
        }
        self.pushed.fetch_add(1, Ordering::Relaxed);
        Ok(id)
    }

    fn insert(g: &mut Inner, mode: SchedulingMode, job: Job) {
        match mode {
            SchedulingMode::Fifo => g.fifo.push_back(job),
            SchedulingMode::Priority => {
                g.prio.push(job);
                // Sort key: higher rank first, then insertion seq for a
                // stable FIFO-within-priority. `sort_by` is stable.
                g.prio.sort_by(|a, b| {
                    b.priority
                        .rank()
                        .cmp(&a.priority.rank())
                        .then(a.id.cmp(&b.id))
                });
            }
            SchedulingMode::Fair => {
                g.per_target[job.target].push_back(job);
            }
        }
    }

    /// Pull the next runnable job. A queued job past its deadline is
    /// returned as `Expired` (never silently discarded) — the engine notes
    /// it and calls pop again for real work.
    pub fn pop(&self) -> Popped {
        let now = Instant::now();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        let job = match self.mode {
            SchedulingMode::Fifo => g.fifo.pop_front(),
            SchedulingMode::Priority => {
                if g.prio.is_empty() {
                    None
                } else {
                    Some(g.prio.remove(0))
                }
            }
            SchedulingMode::Fair => self.pick_fair(&mut g, &active),
        };
        let Some(mut job) = job else {
            return Popped::Empty;
        };
        if job.expired_at(now) {
            job.state = JobState::Expired;
            self.expired.fetch_add(1, Ordering::Relaxed);
            return Popped::Expired(job);
        }
        if let Some(slot) = active.get(job.target) {
            slot.fetch_add(1, Ordering::SeqCst);
        }
        job.state = JobState::Running;
        self.popped.fetch_add(1, Ordering::Relaxed);
        self.inflight.fetch_add(1, Ordering::SeqCst);
        Popped::Job(job)
    }

    fn pick_fair(&self, g: &mut Inner, active: &[AtomicUsize]) -> Option<Job> {
        let n = g.per_target.len();
        if n == 0 {
            return None;
        }
        let start = self.rr.fetch_add(1, Ordering::Relaxed) % n;
        for k in 0..n {
            let t = (start + k) % n;
            if g.per_target[t].is_empty() {
                continue;
            }
            if self.per_target_inflight > 0
                && active.get(t).map(|a| a.load(Ordering::SeqCst)).unwrap_or(0)
                    >= self.per_target_inflight
            {
                continue;
            }
            self.rr.store(t + 1, Ordering::Relaxed);
            return g.per_target[t].pop_front();
        }
        None
    }

    /// Worker finished with the job (any terminal path). Releases the
    /// target's inflight allowance and the global inflight count.
    pub fn release(&self, target: usize) {
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(slot) = active.get(target) {
            slot.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.saturating_sub(1))
            })
            .ok();
        }
        self.inflight
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
                Some(v.saturating_sub(1))
            })
            .ok();
    }

    /// Jobs still queued for one target (budget math: written + queued).
    pub fn count_target(&self, target: usize) -> usize {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match self.mode {
            SchedulingMode::Fifo => g.fifo.iter().filter(|j| j.target == target).count(),
            SchedulingMode::Priority => g.prio.iter().filter(|j| j.target == target).count(),
            SchedulingMode::Fair => g.per_target.get(target).map(|d| d.len()).unwrap_or(0),
        }
    }

    pub fn len(&self) -> usize {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        self.depth_of(&g)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Ordered view of pending jobs for checkpoint snapshots (same order a
    /// worker would see them).
    pub fn pending_snapshot(&self) -> Vec<(String, usize, usize)> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match self.mode {
            SchedulingMode::Fifo => g
                .fifo
                .iter()
                .map(|j| (j.url.to_string(), j.depth, j.target))
                .collect(),
            SchedulingMode::Priority => g
                .prio
                .iter()
                .map(|j| (j.url.to_string(), j.depth, j.target))
                .collect(),
            SchedulingMode::Fair => {
                let mut out = Vec::new();
                let maxlen = g.per_target.iter().map(VecDeque::len).max().unwrap_or(0);
                for i in 0..maxlen {
                    for q in &g.per_target {
                        if let Some(j) = q.get(i) {
                            out.push((j.url.to_string(), j.depth, j.target));
                        }
                    }
                }
                out
            }
        }
    }

    pub fn stats(&self) -> QueueStats {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let per_target = match self.mode {
            SchedulingMode::Fair => g
                .per_target
                .iter()
                .enumerate()
                .map(|(i, q)| (i, q.len()))
                .collect(),
            _ => {
                let mut m: HashMap<usize, usize> = HashMap::new();
                let all: Box<dyn Iterator<Item = &Job>> = match self.mode {
                    SchedulingMode::Fifo => Box::new(g.fifo.iter()),
                    _ => Box::new(g.prio.iter()),
                };
                for j in all {
                    *m.entry(j.target).or_default() += 1;
                }
                m.into_iter().collect()
            }
        };
        let active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        QueueStats {
            mode: self.mode.as_str(),
            depth: self.depth_of(&g),
            max_depth: self.max_depth,
            inflight: self.inflight.load(Ordering::SeqCst),
            per_target_depths: per_target,
            per_target_active: active
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.load(Ordering::SeqCst)))
                .collect(),
            pushed: self.pushed.load(Ordering::Relaxed),
            popped: self.popped.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            expired: self.expired.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueStats {
    pub mode: &'static str,
    pub depth: usize,
    pub max_depth: usize,
    pub inflight: usize,
    pub per_target_depths: Vec<(usize, usize)>,
    pub per_target_active: Vec<(usize, usize)>,
    pub pushed: u64,
    pub popped: u64,
    pub rejected: u64,
    pub expired: u64,
}

/// Convenience for engine call sites that build a job then push.
pub fn make_job(
    url: url::Url,
    depth: usize,
    target: usize,
    priority: JobPriority,
    max_age: Option<Duration>,
) -> Job {
    let j = Job::new(url, depth, target).with_priority(priority);
    match max_age {
        Some(d) => j.with_deadline(d),
        None => j,
    }
}

#[cfg(test)]
mod tests;
