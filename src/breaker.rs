//! Circuit breaker for legitimate network reliability: a closed/open/
//! half-open state machine with configurable failure threshold, recovery
//! window and cooldown, state-transition events, and metrics.
//!
//! This is a *reliability* component: after N consecutive transport/5xx
//! failures to a host, we stop adding load during a cooldown and probe
//! single-file afterwards. It is deliberately NOT a block-evader: an HTTP
//! refusal (401/403/429/451) never trips it — refusals are routed to the
//! politeness backoff in `policy.rs`, which honors them.
//!
//! Time is injected (`now: Instant` parameters) so tests — and
//! `retry_deterministic` runs — advance state machines without sleeping.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

/// Transition log entry kept per breaker (bounded).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BreakerEvent {
    ClosedToOpen,
    OpenToHalfOpen,
    HalfOpenProbeAllowed,
    HalfOpenToClosed,
    HalfOpenToOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BreakerState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug, Clone)]
pub struct BreakerCfg {
    /// Consecutive failures that open the circuit (engine validation
    /// requires >= 2 — one-off failures are noise, not a pattern).
    pub min_consecutive: u32,
    /// How long the circuit stays fully open before a probe is allowed.
    pub cooldown: Duration,
    /// A half-open probe must succeed this many times in a row to close.
    pub recovery_successes: u32,
}

impl Default for BreakerCfg {
    fn default() -> Self {
        Self {
            min_consecutive: 5,
            cooldown: Duration::from_secs(30),
            recovery_successes: 1,
        }
    }
}

/// Mutable core guarded by one mutex; atomics expose the hot read path
/// (`state()`) lock-free for metrics snapshots.
#[derive(Debug)]
struct Inner {
    state: BreakerState,
    consecutive_failures: u32,
    consecutive_successes: u32,
    opened_at: Option<Instant>,
    /// Only one probe in flight at a time — half-open is a *single* request
    /// test, not a thundering reopening.
    probe_in_flight: bool,
    transitions: u64,
    last_event: Option<BreakerEvent>,
}

pub struct Breaker {
    cfg: BreakerCfg,
    inner: Mutex<Inner>,
    state_atomic: AtomicU32,
    total_opens: AtomicU64,
    total_rejections: AtomicU64,
    clock_start: Instant,
}

/// Answer to "may I send this request now?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permit {
    Allowed,
    /// Circuit open; retry after `retry_in` (never before — callers must
    /// not treat this as a failure of their own request).
    Open {
        retry_in: Duration,
    },
    /// This caller *is* the half-open probe; a failure here reopens.
    Probe,
}

impl Breaker {
    pub fn new(cfg: BreakerCfg) -> Self {
        Self {
            cfg,
            inner: Mutex::new(Inner {
                state: BreakerState::Closed,
                consecutive_failures: 0,
                consecutive_successes: 0,
                opened_at: None,
                probe_in_flight: false,
                transitions: 0,
                last_event: None,
            }),
            state_atomic: AtomicU32::new(0),
            total_opens: AtomicU64::new(0),
            total_rejections: AtomicU64::new(0),
            clock_start: Instant::now(),
        }
    }

    pub fn state(&self) -> BreakerState {
        match self.state_atomic.load(Ordering::Relaxed) {
            1 => BreakerState::Open,
            2 => BreakerState::HalfOpen,
            _ => BreakerState::Closed,
        }
    }

    fn set_state(&self, g: &mut Inner, s: BreakerState, ev: BreakerEvent) {
        g.state = s;
        g.transitions += 1;
        g.last_event = Some(ev);
        self.state_atomic.store(
            match s {
                BreakerState::Closed => 0,
                BreakerState::Open => 1,
                BreakerState::HalfOpen => 2,
            },
            Ordering::Relaxed,
        );
        if matches!(s, BreakerState::Open) {
            self.total_opens.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Call before issuing a request to this host. `now` injected for
    /// deterministic tests; production passes `Instant::now()`.
    pub fn permit(&self, now: Instant) -> Permit {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match g.state {
            BreakerState::Closed => Permit::Allowed,
            BreakerState::Open => {
                let waited = g.opened_at.map(|t| now.saturating_duration_since(t));
                match waited {
                    Some(w) if w >= self.cfg.cooldown => {
                        self.set_state(
                            &mut g,
                            BreakerState::HalfOpen,
                            BreakerEvent::OpenToHalfOpen,
                        );
                        g.probe_in_flight = true;
                        g.consecutive_successes = 0;
                        Permit::Probe
                    }
                    Some(w) => {
                        self.total_rejections.fetch_add(1, Ordering::Relaxed);
                        Permit::Open {
                            retry_in: self.cfg.cooldown.saturating_sub(w),
                        }
                    }
                    None => {
                        // Open without a timestamp is unreachable by
                        // construction; fail safe by allowing (never wedge).
                        Permit::Allowed
                    }
                }
            }
            BreakerState::HalfOpen => {
                if g.probe_in_flight {
                    self.total_rejections.fetch_add(1, Ordering::Relaxed);
                    Permit::Open {
                        retry_in: self.cfg.cooldown / 4,
                    }
                } else {
                    g.probe_in_flight = true;
                    Permit::Probe
                }
            }
        }
    }

    /// Successful response. In half-open this counts toward recovery.
    pub fn note_success(&self, now: Instant) -> Option<BreakerEvent> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.consecutive_failures = 0;
        match g.state {
            BreakerState::HalfOpen => {
                g.consecutive_successes += 1;
                g.probe_in_flight = false;
                if g.consecutive_successes >= self.cfg.recovery_successes {
                    self.set_state(&mut g, BreakerState::Closed, BreakerEvent::HalfOpenToClosed);
                    g.opened_at = None;
                    Some(BreakerEvent::HalfOpenToClosed)
                } else {
                    None
                }
            }
            _ => {
                let _ = now;
                g.consecutive_successes = 0;
                None
            }
        }
    }

    /// Hand a taken probe slot back without a verdict (the engine grabbed a
    /// probe permit but the job ended before any fetch — robots-denied,
    /// deduped, abandoned). Without this, half-open could wait forever for
    /// a note that never comes and the host would wedge.
    pub fn release_probe(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if g.state == BreakerState::HalfOpen {
            g.probe_in_flight = false;
        }
    }

    /// A transport/5xx failure. Opens the circuit at threshold (closed) or
    /// slams it shut again mid-probe (half-open). `now` is injected like in
    /// `permit` so the whole state machine runs on one clock.
    pub fn note_failure(&self, now: Instant) -> Option<BreakerEvent> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.consecutive_successes = 0;
        match g.state {
            BreakerState::Closed => {
                g.consecutive_failures += 1;
                if g.consecutive_failures >= self.cfg.min_consecutive {
                    g.opened_at = Some(now);
                    g.probe_in_flight = false;
                    self.set_state(&mut g, BreakerState::Open, BreakerEvent::ClosedToOpen);
                    Some(BreakerEvent::ClosedToOpen)
                } else {
                    None
                }
            }
            BreakerState::HalfOpen => {
                g.opened_at = Some(now);
                g.probe_in_flight = false;
                g.consecutive_failures = self.cfg.min_consecutive; // stays saturated
                self.set_state(&mut g, BreakerState::Open, BreakerEvent::HalfOpenToOpen);
                Some(BreakerEvent::HalfOpenToOpen)
            }
            BreakerState::Open => None,
        }
    }

    /// Explicit reset (e.g. a run boundary must not inherit last run's
    /// open circuit for a different crawl context).
    pub fn reset(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        *g = Inner {
            state: BreakerState::Closed,
            consecutive_failures: 0,
            consecutive_successes: 0,
            opened_at: None,
            probe_in_flight: false,
            transitions: g.transitions,
            last_event: g.last_event,
        };
        self.state_atomic.store(0, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> BreakerSnapshot {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        BreakerSnapshot {
            state: g.state,
            consecutive_failures: g.consecutive_failures,
            transitions: g.transitions,
            total_opens: self.total_opens.load(Ordering::Relaxed),
            total_rejections: self.total_rejections.load(Ordering::Relaxed),
            last_event: g.last_event,
            opened_ms_ago: g
                .opened_at
                .map(|t| t.elapsed().as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// Uptime-relative clock start, exposed so the runtime snapshot can
    /// render "open for Ns" without holding the mutex.
    pub fn started(&self) -> Instant {
        self.clock_start
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BreakerSnapshot {
    pub state: BreakerState,
    pub consecutive_failures: u32,
    pub transitions: u64,
    pub total_opens: u64,
    pub total_rejections: u64,
    pub last_event: Option<BreakerEvent>,
    pub opened_ms_ago: u64,
}

#[cfg(test)]
mod tests;
