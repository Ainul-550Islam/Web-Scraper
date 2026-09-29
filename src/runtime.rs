//! Runtime snapshot: the single object that can answer "what is the engine
//! doing" without interrogating five subsystems.
//!
//! The [`Runtime`] owns the run's observability plane — event bus, resource
//! accounting, per-host transport breakers, supervisor handle and (while a
//! crawl is live) the queue — and renders one JSON snapshot consumed by
//! `GET /runtime`, written by the optional events log, and asserted by
//! tests. It carries no control authority: every consumer reads.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use crate::accounting::Accounting;
use crate::breaker::{Breaker, BreakerCfg};
use crate::config::Config;
use crate::events::{Bus, EventKind};
use crate::metrics::Metrics;
use crate::retry::RetryBudget;
use crate::sched::CrawlQueue;
use crate::supervisor::Supervisor;

/// Bus ring capacity: 8 192 events is ~1-2 MB worst case of JSON values —
/// generous for post-mortem tails, negligible for a crawl process.
pub const BUS_CAP: usize = 8192;

/// Process-level lifecycle of a [`Runtime`]. Deliberately coarse and closed:
/// the detailed per-worker state lives in the supervisor snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleState {
    /// constructed, not attached yet (no workers scheduled)
    Starting,
    /// accepting work
    Running,
    /// stop requested: intake closed, workers draining to a safe point
    Draining,
    /// a caller marked the run degraded (embedder/ops signal)
    Degraded,
    /// final state after `end_shutdown`
    Stopped,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Draining => "draining",
            Self::Degraded => "degraded",
            Self::Stopped => "stopped",
        }
    }
}

pub struct Runtime {
    pub bus: Arc<Bus>,
    pub acct: Arc<Accounting>,
    pub metrics: Arc<Metrics>,
    /// Supervisor handle if this run supervises workers (engine runs do;
    /// auxiliary runtimes don't need one).
    supervisor: Mutex<Option<Arc<Supervisor>>>,
    /// Live queue reference (set when a crawl is in flight; cleared at run
    /// end so snapshots never report a stale queue).
    queue: Mutex<Option<Arc<CrawlQueue>>>,
    breakers: Mutex<BTreeMap<String, Arc<Breaker>>>,
    breaker_cfg: Option<BreakerCfg>,
    budget: Option<Arc<RetryBudget>>,
    /// begin_shutdown idempotence latch (event pair must never duplicate).
    started_marked: std::sync::atomic::AtomicBool,
    run_name: String,
    started: Instant,
    /// unix seconds of `new()` — human-facing start stamp (wall clock)
    start_unix: u64,
    /// 0 Starting | 1 Running | 2 Draining | 3 Stopped | 4 Degraded
    lifecycle: std::sync::atomic::AtomicU8,
    drain_at_ms: std::sync::atomic::AtomicU64,
    drain_marked: std::sync::atomic::AtomicBool,
    drain_reason: Mutex<Option<String>>,
}

// Small helper alias so the hot paths below stay readable.
type Mutex<T> = std::sync::Mutex<T>;

impl Runtime {
    pub fn new(cfg: &Config, metrics: Arc<Metrics>) -> Arc<Self> {
        let breaker_cfg = cfg.transport_breaker.as_ref().map(|b| BreakerCfg {
            min_consecutive: b.min_consecutive,
            cooldown: std::time::Duration::from_secs(b.cooldown_secs.max(1)),
            recovery_successes: b.recovery_successes.max(1),
        });
        Arc::new(Self {
            bus: Bus::new(BUS_CAP),
            acct: Accounting::new(),
            metrics,
            supervisor: Mutex::new(None),
            queue: Mutex::new(None),
            breakers: Mutex::new(BTreeMap::new()),
            breaker_cfg,
            budget: (cfg.scheduling.retry_budget_per_min > 0)
                .then(|| Arc::new(RetryBudget::new(cfg.scheduling.retry_budget_per_min))),
            started_marked: std::sync::atomic::AtomicBool::new(false),
            run_name: cfg.name.clone(),
            started: Instant::now(),
            start_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            lifecycle: std::sync::atomic::AtomicU8::new(0),
            drain_at_ms: std::sync::atomic::AtomicU64::new(0),
            drain_marked: std::sync::atomic::AtomicBool::new(false),
            drain_reason: Mutex::new(None),
        })
    }

    pub fn attach_supervisor(&self, sup: Arc<Supervisor>) {
        *self.supervisor.lock().unwrap_or_else(|p| p.into_inner()) = Some(sup);
        // Starting → Running (never backwards; a re-attach keeps Draining/
        // Degraded/Stopped authoritative).
        let _ = self.lifecycle.compare_exchange(
            0,
            1,
            std::sync::atomic::Ordering::Relaxed,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
    pub fn attach_queue(&self, q: Arc<CrawlQueue>) {
        *self.queue.lock().unwrap_or_else(|p| p.into_inner()) = Some(q);
    }
    pub fn detach_queue(&self) {
        *self.queue.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// The run's shared retry budget (None = unlimited). Workers hand this
    /// to every `execute`, so the cap is enforced across all of them at
    /// once, not per-worker (which would scale with concurrency and leak).
    pub fn retry_budget(&self) -> Option<Arc<RetryBudget>> {
        self.budget.clone()
    }

    /// True only while a *stop* is in progress (the supervisor's cancel
    /// token — the same flag the ctrl-c handler sets), not merely because
    /// intake closed. Snapshot status wording hangs off this, so it must
    /// not read "draining" for a crawl that is actively fetching.
    pub fn is_draining(&self) -> bool {
        self.supervisor
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|s| s.is_draining())
            .unwrap_or(false)
    }

    /// Breaker for a host, if the feature is configured. The map grows one
    /// entry per distinct host contacted — crawl-scoped by construction
    /// (hosts are also capped by politeness state the engine already keeps).
    pub fn breaker_for(&self, host: &str) -> Option<Arc<Breaker>> {
        let cfg = self.breaker_cfg.clone()?;
        let mut g = self.breakers.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(b) = g.get(host) {
            return Some(b.clone());
        }
        let b = Arc::new(Breaker::new(cfg));
        g.insert(host.to_string(), b.clone());
        Some(b)
    }

    /// Lifecycle coordinator (this is the state enum; `snapshot()` remains
    /// exactly what `GET /runtime` has always returned — the lifecycle view
    /// is a SEPARATE read side via [`Runtime::lifecycle_json`], so public API
    /// contracts cannot drift by accident).
    pub fn lifecycle(&self) -> LifecycleState {
        let raw = self.lifecycle.load(std::sync::atomic::Ordering::Relaxed);
        match raw {
            1 => LifecycleState::Running,
            2 => LifecycleState::Draining,
            3 => LifecycleState::Stopped,
            4 => LifecycleState::Degraded,
            _ => LifecycleState::Starting,
        }
    }

    /// Uptime in ms since `Runtime::new` (monotonic clock; no wall-clock
    /// jumps, never negative).
    pub fn uptime_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Wall-clock start of this runtime (unix seconds) — for humans; uptime
    /// itself stays monotonic.
    pub fn started_unix(&self) -> u64 {
        self.start_unix
    }

    /// Embedder/diagnostic hook: mark the run degraded (e.g. an operator
    /// pausing intake). Refused once draining/stopped — shutdown ordering
    /// must not be reshuffled by side channels.
    pub fn mark_degraded(&self) -> bool {
        self.lifecycle
            .compare_exchange(
                1,
                4,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
    }
    pub fn clear_degraded(&self) -> bool {
        self.lifecycle
            .compare_exchange(
                4,
                1,
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
            )
            .is_ok()
    }

    /// Record why a drain began (`signal`/`schedule`/`admin`/`fault` are the
    /// engine's real causes; free text is refused — bounded vocabulary only).
    pub fn note_shutdown_reason(&self, reason: &str) {
        let bounded = match reason {
            "signal" | "schedule" | "admin" | "fault" => reason,
            _ => "signal",
        };
        *self.drain_reason.lock().unwrap_or_else(|p| p.into_inner()) = Some(bounded.to_string());
    }

    /// Standalone lifecycle view (NOT part of `snapshot()`).
    pub fn lifecycle_json(&self) -> Value {
        use std::sync::atomic::Ordering::Relaxed;
        let at = self.drain_at_ms.load(Relaxed);
        let drain = self
            .drain_marked
            .load(Relaxed)
            .then_some(serde_json::json!({
                "at_ms": at,
                "reason": self.drain_reason.lock().unwrap_or_else(|p| p.into_inner()).clone(),
            }));
        serde_json::json!({
            "state": self.lifecycle().as_str(),
            "uptime_ms": self.uptime_ms(),
            "started_unix": self.started_unix(),
            "drain": drain,
        })
    }

    /// Publish + count in one call (the engine's hot-path shape).
    pub fn emit(&self, kind: EventKind, job: Option<u64>, detail: Value) -> u64 {
        self.bus.publish(kind, job, detail)
    }

    /// Full runtime snapshot for `GET /runtime`.
    pub fn snapshot(&self, running: bool) -> Value {
        let queue = self
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|q| serde_json::to_value(q.stats()).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let supervisor = self
            .supervisor
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|s| serde_json::to_value(s.snapshot()).unwrap_or(Value::Null))
            .unwrap_or(Value::Null);
        let breakers: BTreeMap<String, Value> = {
            let g = self.breakers.lock().unwrap_or_else(|p| p.into_inner());
            g.iter()
                .filter(|(_, b)| b.state() != crate::breaker::BreakerState::Closed)
                .map(|(h, b)| {
                    (
                        h.clone(),
                        serde_json::to_value(b.snapshot()).unwrap_or(Value::Null),
                    )
                })
                .collect()
        };
        let acct = self.acct.snapshot();
        json!({
            "engine": {
                "run": self.run_name,
                "status": if self.is_draining() {
                    "draining"
                } else if running {
                    "running"
                } else {
                    "idle"
                },
                "uptime_ms": self.started.elapsed().as_millis() as u64,
            },
            "accounting": serde_json::to_value(&acct).unwrap_or(Value::Null),
            "queue": queue,
            "workers": supervisor,
            "breakers": breakers,
            "metrics": self.metrics.display_json(),
            "events": {
                "published": self.bus.stats().published,
                "dropped": self.bus.stats().dropped,
                "ring_len": self.bus.stats().ring_len,
                "last_seq": self.bus.stats().last_seq,
            },
        })
    }

    /// Emit the drain-phase marker (once). Called when the engine closes
    /// intake and starts waiting on workers — "no more pushes; run to
    /// dry". `"draining"` status is separate: it lights up only when a
    /// real stop was requested (the supervisor's cancel token).
    pub fn begin_shutdown(&self) {
        use std::sync::atomic::Ordering;
        // entering drain: Running/Starting/Degraded → Draining. Stopped is
        // FINAL (a late second stop signal must not resurrect a finished run).
        let prev = self.lifecycle.load(Ordering::Relaxed);
        if prev != 2 && prev != 3 {
            self.lifecycle.store(2, Ordering::Relaxed);
            // uptime_ms may legitimately be 0 in the first millisecond, so a
            // separate latch marks "drain recorded" (keeps first timestamp).
            if !self.drain_marked.swap(true, Ordering::Relaxed) {
                self.drain_at_ms.store(self.uptime_ms(), Ordering::Relaxed);
            }
        }
        if !self.started_marked.swap(true, Ordering::Relaxed) {
            self.emit(
                EventKind::ShutdownStarted,
                None,
                json!({"run": self.run_name}),
            );
        }
    }

    /// Final event for run end.
    pub fn end_shutdown(&self, summary: &Value) {
        self.lifecycle
            .store(3, std::sync::atomic::Ordering::Relaxed);
        self.emit(
            EventKind::ShutdownCompleted,
            None,
            json!({"run": self.run_name, "summary": summary}),
        );
    }
}

/// JSONL events-log task body: streams bus events to `path` (one compact
/// JSON object per line) from seq 1 until the bus closes at run end.
/// Returns the number of lines written. Uses tokio fs so a current-thread
/// test runtime never blocks on disk.
pub async fn events_log_writer(bus: Arc<Bus>, path: std::path::PathBuf) -> anyhow::Result<u64> {
    use tokio::io::AsyncWriteExt;
    if let Some(p) = path.parent() {
        let _ = tokio::fs::create_dir_all(p).await;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await
        .map_err(|e| anyhow::anyhow!("events log {}: {e}", path.display()))?;
    let mut cursor = 0u64;
    let mut written = 0u64;
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    loop {
        let Some(ev) = bus.next_after(cursor).await else {
            break;
        };
        cursor = ev.seq;
        let line = serde_json::to_string(&ev)?;
        buf.extend_from_slice(line.as_bytes());
        buf.push(b'\n');
        written += 1;
        if buf.len() >= 32 * 1024 {
            file.write_all(&buf).await?;
            buf.clear();
        }
    }
    if !buf.is_empty() {
        file.write_all(&buf).await?;
    }
    file.flush().await?;
    Ok(written)
}

#[cfg(test)]
mod tests;

/// Indirection so the admin server can be built before the first run and
/// still find the live runtime: engine sets it, admin reads it, both sides
/// tolerate None (idle engine => `"error":"no run active"` snapshot).
#[derive(Clone, Default)]
pub struct RuntimeSlot {
    inner: Arc<Mutex<Option<Arc<Runtime>>>>,
}

impl RuntimeSlot {
    pub fn set(&self, rt: Arc<Runtime>) {
        *self.inner.lock().unwrap_or_else(|p| p.into_inner()) = Some(rt);
    }
    pub fn get(&self) -> Option<Arc<Runtime>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn is_empty(&self) -> bool {
        self.get().is_none()
    }
}
