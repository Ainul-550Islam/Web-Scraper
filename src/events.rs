//! Internal event bus: a bounded, async-safe, cancellation-aware event
//! ring that the engine publishes to and operators (admin API, JSONL
//! event log) read from.
//!
//! Design choices, each load-bearing for a long-running crawl:
//! * **Bounded by construction** — fixed capacity ring; publishers never
//!   block and never grow memory. Overflow drops the *oldest* events and
//!   counts them; a subscriber detects the gap via `dropped`.
//! * **Async-safe** — readers wait on a `tokio::sync::Notify`; no polling
//!   loop, no busy CPU, and a slow reader can only lag (and observe the
//!   drop counter), never stall a crawl.
//! * **Cancellation-aware** — `close()` releases every waiter with
//!   `None` so subscribers join cleanly on drain.
//! * **Testable** — no global state, injectable clock stamping, the cursor
//!   API lets a test read exactly the next event.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;
use tokio::sync::Notify;

/// The catalog of engine events. Kept flat (string kind + JSON detail)
/// because these are *observation*, not control: adding a field to a
/// variant must never ripple into the crawl's hot paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    JobSubmitted,
    JobStarted,
    RequestStarted,
    RequestFinished,
    RetryScheduled,
    ProxySelected,
    ProxyFailure,
    ExtractionCompleted,
    SinkWriteCompleted,
    JobCompleted,
    JobFailed,
    ShutdownStarted,
    ShutdownCompleted,
    // Engine-layer extras justified by their consumers:
    /// A queued job passed its deadline before a worker took it.
    JobExpired,
    /// The bounded queue rejected a push (backpressure signal).
    QueueOverflow,
    /// A per-host transport circuit breaker opened (stop hammering).
    BreakerOpened,
    /// A per-host transport circuit breaker closed again.
    BreakerClosed,
    /// A record violated its target's declared schema and was quarantined
    /// instead of written (strict mode) — the job itself still succeeded.
    RecordQuarantined,
    /// Operator-plane action (admin API or CLI): run lifecycle markers,
    /// alert acks, event-log toggles. Observation only — never control.
    OperatorAction,
    /// A bounded resource budget refused work: the run-scoped record cap
    /// tripped (and the scheduler began draining), or a proxy egress hit
    /// its in-flight ceiling. Emitted once per tripped cap, never per
    /// denied request.
    BudgetExhausted,
    /// A browser worker opened or reused an isolated WebDriver session.
    BrowserSessionCreated,
    /// A declarative browser interaction completed.
    BrowserActionCompleted,
    /// The page presented an observable challenge; policy decided to stop/review.
    BrowserChallengeDetected,
    /// A selector tree produced a bounded set of preview records.
    SelectorPreviewCompleted,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::JobSubmitted => "job_submitted",
            EventKind::JobStarted => "job_started",
            EventKind::RequestStarted => "request_started",
            EventKind::RequestFinished => "request_finished",
            EventKind::RetryScheduled => "retry_scheduled",
            EventKind::ProxySelected => "proxy_selected",
            EventKind::ProxyFailure => "proxy_failure",
            EventKind::ExtractionCompleted => "extraction_completed",
            EventKind::SinkWriteCompleted => "sink_write_completed",
            EventKind::BudgetExhausted => "budget_exhausted",
            EventKind::JobCompleted => "job_completed",
            EventKind::JobFailed => "job_failed",
            EventKind::ShutdownStarted => "shutdown_started",
            EventKind::ShutdownCompleted => "shutdown_completed",
            EventKind::JobExpired => "job_expired",
            EventKind::QueueOverflow => "queue_overflow",
            EventKind::BreakerOpened => "breaker_opened",
            EventKind::BreakerClosed => "breaker_closed",
            EventKind::RecordQuarantined => "record_quarantined",
            EventKind::OperatorAction => "operator_action",
            EventKind::BrowserSessionCreated => "browser_session_created",
            EventKind::BrowserActionCompleted => "browser_action_completed",
            EventKind::BrowserChallengeDetected => "browser_challenge_detected",
            EventKind::SelectorPreviewCompleted => "selector_preview_completed",
        }
    }
}

/// One event. `seq` is dense per bus (monotonic, +1 per *published* event
/// including dropped-from-ring ones), so subscribers can detect gaps.
#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub seq: u64,
    pub ts: u64,
    pub kind: EventKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<u64>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

/// Bounded publish/observe bus. Clone via `Arc` handles; every engine
/// component holds the same bus.
#[derive(Debug)]
pub struct Bus {
    inner: Mutex<Ring>,
    notify: Notify,
    seq: AtomicU64,
    dropped: AtomicU64,
    published: AtomicU64,
    closed: AtomicBool,
    cap: usize,
}

#[derive(Debug)]
struct Ring {
    events: VecDeque<Event>,
}

impl Bus {
    pub fn new(cap: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Ring {
                events: VecDeque::with_capacity(cap.min(64)),
            }),
            notify: Notify::new(),
            seq: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            published: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            cap: cap.max(16),
        })
    }

    /// Publish one event. O(1), lock held only for a deque push; safe to
    /// call from any task, any thread, hot paths included.
    pub fn publish(&self, kind: EventKind, job: Option<u64>, mut detail: Value) -> u64 {
        crate::config::redact_observation(&mut detail);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.published.fetch_add(1, Ordering::Relaxed);
        let ev = Event {
            seq,
            ts: now_secs(),
            kind,
            job,
            detail,
        };
        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if g.events.len() >= self.cap {
                g.events.pop_front();
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            g.events.push_back(ev);
        }
        // wake_any semantics: Notify::notify_waiters releases all current
        // waiters; late readers fall through the cursor check immediately.
        self.notify.notify_waiters();
        seq
    }

    /// The last `n` events, oldest-first.
    pub fn tail(&self, n: usize) -> Vec<Event> {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let skip = g.events.len().saturating_sub(n);
        g.events.iter().skip(skip).cloned().collect()
    }

    pub fn stats(&self) -> BusStats {
        BusStats {
            published: self.published.load(Ordering::Relaxed),
            dropped: self.dropped.load(Ordering::Relaxed),
            ring_len: self
                .inner
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .events
                .len(),
            cap: self.cap,
            last_seq: self.seq.load(Ordering::Relaxed),
            closed: self.closed.load(Ordering::Relaxed),
        }
    }

    /// Next event strictly after `cursor`, waiting asynchronously for one.
    /// `None` when the bus is closed (drain complete) — subscribers treat
    /// that as shutdown and flush.
    pub async fn next_after(&self, cursor: u64) -> Option<Event> {
        loop {
            let found = {
                let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                // First retained event past the cursor. If the consumer
                // lagged and its next seq was already dropped from the
                // ring, `find` naturally rejoins at the oldest retained
                // one — continuous flow beats a gap stall.
                g.events.iter().find(|e| e.seq > cursor).cloned()
            };
            if let Some(ev) = found {
                return Some(ev);
            }
            // Drain-before-done: closing the bus must not strand the final
            // events of a run (the shutdown pair is exactly what an audit
            // of "did everything end cleanly" reads).
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            self.notify.notified().await;
        }
    }

    /// Release waiters (`None` from `next_after`) and stop accepting
    /// meaningful publishes (publish still works after close — events just
    /// stop being *waited* for; a closing run's tail events are picked up
    /// by the final drain pass of consumers).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.notify.notify_waiters();
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BusStats {
    pub published: u64,
    pub dropped: u64,
    pub ring_len: usize,
    pub cap: usize,
    pub last_seq: u64,
    pub closed: bool,
}

/// One publish site's convenience view (job-scoped events).
pub trait PublishExt {
    fn publish_job(&self, kind: EventKind, job_id: u64, detail: Value) -> u64;
}
impl PublishExt for Arc<Bus> {
    fn publish_job(&self, kind: EventKind, job_id: u64, detail: Value) -> u64 {
        self.publish(kind, Some(job_id), detail)
    }
}

pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
