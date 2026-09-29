//! Job model and lifecycle: the unit the scheduler, supervisor and event
//! bus all agree on.
//!
//! The crawl engine historically passed a bare `(url, depth, target)` tuple
//! through its queue; that shape is now this module's [`Job`], extended with
//! an identity, a state machine, retry metadata, an optional deadline and
//! structured status. The state machine is *enforced* ([`Job::transition`]
//! rejects illegal moves), because silently-wrong lifecycle state is how
//! supervisory systems lie about themselves.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

/// Process-unique job identity. Monotonic per process; `0` is reserved for
/// "not a scheduled job" (e.g. robots/sitemap helpers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct JobId(pub u64);

static JOB_SEQ: AtomicU64 = AtomicU64::new(1);

impl JobId {
    /// Next identity in process order. Never wraps within any practical run
    /// (u64), and a wrap is still only identity reuse, never a safety issue.
    pub fn next() -> Self {
        JobId(JOB_SEQ.fetch_add(1, Ordering::Relaxed))
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "job-{}", self.0)
    }
}

/// Scheduling class. `High` drains before `Normal` only under
/// `scheduling.mode = "priority"`; FIFO ignores priority (documented, not
/// silently applied).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum JobPriority {
    High,
    #[default]
    Normal,
    Low,
}

impl JobPriority {
    /// Heap key: numerically larger drains first.
    pub(crate) fn rank(self) -> u8 {
        match self {
            JobPriority::High => 2,
            JobPriority::Normal => 1,
            JobPriority::Low => 0,
        }
    }
}

/// Lifecycle states. Terminal states never transition again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// Accepted into the queue, not yet picked by a worker.
    #[default]
    Pending,
    /// Owned by a worker; a deadline still applies.
    Running,
    /// Finished and written (or deliberately skipped: dedup/304 — still a
    /// success for the queue's books; the record-level truth is metrics).
    Completed,
    /// Finished without writing, for a recorded reason (fetch failure,
    /// breaker-open, sink error).
    Failed,
    /// Cancelled by drain/shutdown before its next attempt point.
    Cancelled,
    /// Its deadline elapsed before a worker could start it.
    Expired,
}

impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobState::Completed | JobState::Failed | JobState::Cancelled | JobState::Expired
        )
    }
    /// The only legal moves; anything else is a bug we want surfaced, not
    /// absorbed.
    pub fn can_transition(from: JobState, to: JobState) -> bool {
        use JobState as J;
        matches!(
            (from, to),
            (J::Pending, J::Running)
                | (J::Pending, J::Cancelled)
                | (J::Pending, J::Expired)
                | (J::Running, J::Completed)
                | (J::Running, J::Failed)
                | (J::Running, J::Cancelled)
        )
    }
}

/// One retry record inside a job's attempt history (bounded; see
/// [`Job::note_retry`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub attempt: u32,
    pub delay_ms: u64,
    /// Category string from [`crate::retry::RetryReason::as_str`].
    pub reason: String,
}

/// Aggregate retry accounting carried with a job through the queue
/// (checkpoint round-trips keep it).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RetryMeta {
    /// Attempts consumed so far (0 = never tried).
    pub attempts: u32,
    pub history: Vec<RetryAttempt>,
}

impl RetryMeta {
    pub const HISTORY_CAP: usize = 8;
    pub fn note(&mut self, attempt: u32, delay_ms: u64, reason: &str) {
        self.attempts = self.attempts.max(attempt);
        if self.history.len() >= Self::HISTORY_CAP {
            self.history.remove(0);
        }
        self.history.push(RetryAttempt {
            attempt,
            delay_ms,
            reason: reason.to_string(),
        });
    }
}

/// Why a job ended; carried into events and the recent-failure ring.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobError {
    pub reason: String,
    /// Which category the retry layer assigned (when it got that far).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub attempts_used: u32,
}

/// Structured success report for one job.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct JobResult {
    /// Was a record written to the sink for this job?
    pub written: bool,
    /// Record dropped by fingerprint dedup (still a completion).
    pub deduped: bool,
    /// 304 revalidation confirmation (still a completion).
    pub revalidated: bool,
    pub egress_ms: u64,
    pub bytes: u64,
}

/// A unit of crawl work: what to fetch, for which target, with its
/// lifecycle bookkeeping. The engine's queue holds these.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: JobId,
    pub url: Url,
    pub depth: usize,
    pub target: usize,
    #[serde(default)]
    pub priority: JobPriority,
    #[serde(default)]
    pub state: JobState,
    /// Monotonic intake clock anchor (queue wait + deadline math). Process-
    /// local only: a restored job from a checkpoint legitimately restarts
    /// its age at the moment of re-queueing.
    #[serde(skip, default = "Instant::now")]
    pub submitted_at: Instant,
    /// Absolute deadline; `None` = none. Enforced by the scheduler at pop
    /// (a job that waited past its deadline expires instead of crawling
    /// stale-budget work after a long block).
    #[serde(skip)]
    pub deadline: Option<Instant>,
    #[serde(default)]
    pub retry: RetryMeta,
    #[serde(skip)]
    pub result: Option<JobResult>,
    #[serde(skip)]
    pub error: Option<JobError>,
}

impl Job {
    pub fn new(url: Url, depth: usize, target: usize) -> Self {
        Self {
            id: JobId::next(),
            url,
            depth,
            target,
            priority: JobPriority::Normal,
            state: JobState::Pending,
            submitted_at: Instant::now(),
            deadline: None,
            retry: RetryMeta::default(),
            result: None,
            error: None,
        }
    }

    pub fn with_deadline(mut self, since_submit: Duration) -> Self {
        self.deadline = Some(self.submitted_at + since_submit);
        self
    }

    pub fn with_priority(mut self, p: JobPriority) -> Self {
        self.priority = p;
        self
    }

    pub fn expired_at(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|d| now >= d)
    }

    /// Enforced lifecycle move. Illegal transitions are *returned*, not
    /// panicked — the callers are runtime paths.
    pub fn transition(&mut self, to: JobState) -> Result<(), TransitionError> {
        if !JobState::can_transition(self.state, to) {
            return Err(TransitionError {
                id: self.id,
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }

    pub fn complete(&mut self, result: JobResult) {
        self.result = Some(result);
        // Best-effort by design: illegal state here means an upstream bug
        // we already surface via other signals (metrics divergence); it must
        // never corrupt the crawl result itself.
        let _ = self.transition(JobState::Completed);
    }

    pub fn fail(&mut self, error: JobError) {
        self.error = Some(error);
        let _ = self.transition(JobState::Failed);
    }

    /// Wire/status projection for logs, events and the admin snapshot.
    pub fn status_report(&self) -> JobStatus {
        JobStatus {
            id: self.id,
            url: self.url.to_string(),
            target: self.target,
            depth: self.depth,
            state: self.state,
            waited_ms: self
                .submitted_at
                .elapsed()
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX),
            priority: self.priority,
            attempts: self.retry.attempts,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct JobStatus {
    pub id: JobId,
    pub url: String,
    pub target: usize,
    pub depth: usize,
    pub state: JobState,
    pub waited_ms: u64,
    pub priority: JobPriority,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionError {
    pub id: JobId,
    pub from: JobState,
    pub to: JobState,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "illegal job transition {:?} -> {:?} for {}",
            self.from, self.to, self.id
        )
    }
}
impl std::error::Error for TransitionError {}

#[cfg(test)]
mod tests;
