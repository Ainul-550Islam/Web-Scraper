//! Retry policy: categorized reasons, exponential backoff with bounded
//! jitter (exact/deterministic in test mode), retry budgets and per-scope
//! attempt limits.
//!
//! The delay formula is the one the engine has shipped since v0.1 —
//! `base * 2^min(attempt-1, 5) + jitter(0..=250)ms`, capped per-wait —
//! now in one place so the fetch loop, budgets and tests share it. Changing
//! transport pacing is a *behavior* change; keeping the math identical is
//! why this module exists before anyone re-derives it inline somewhere.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

use serde::Serialize;

/// Why we are even allowed to try again. Categories drive accounting and
/// events; they never change the politeness outcome (a 403 is an answer,
/// see [`RetryDecision::NoRetry`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryReason {
    /// DNS/connect/read failure, connection reset.
    Transport,
    /// Server answered 5xx — retryable, the site asked for nothing else.
    Server5xx,
    /// Body read died mid-stream.
    BodyRead,
    /// The client-level timeout fired.
    Timeout,
    /// Our own absolute deadline (queue wait + attempts) ran out.
    Deadline,
    /// Catch-all for messages we cannot classify (still counted).
    Unclassified,
}

impl RetryReason {
    pub fn as_str(self) -> &'static str {
        match self {
            RetryReason::Transport => "transport",
            RetryReason::Server5xx => "server-5xx",
            RetryReason::BodyRead => "body-read",
            RetryReason::Timeout => "timeout",
            RetryReason::Deadline => "deadline",
            RetryReason::Unclassified => "unclassified",
        }
    }

    /// Classify from the fetcher's failure string. Substring rules mirror
    /// how those strings are produced (`try_once` formats); ordering puts
    /// the more specific signatures first.
    pub fn classify(msg: &str) -> RetryReason {
        let m = msg.to_ascii_lowercase();
        if m.contains("server status 5") {
            RetryReason::Server5xx
        } else if m.contains("deadline") {
            RetryReason::Deadline
        } else if m.starts_with("body read") || m.contains("body read") {
            RetryReason::BodyRead
        } else if m.contains("timed out") || m.contains("timeout") {
            RetryReason::Timeout
        } else if m.contains("connect")
            || m.contains("connection")
            || m.contains("dns")
            || m.contains("reset")
            || m.contains("refused")
            || m.contains("broken pipe")
        {
            RetryReason::Transport
        } else {
            RetryReason::Unclassified
        }
    }
}

/// The retry loop's single decision point, so policy is testable without a
/// socket.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryDecision {
    /// Sleep `delay`, then attempt `next_attempt`.
    Retry {
        next_attempt: u32,
        delay: Duration,
        reason: RetryReason,
    },
    /// Give up after exhausting `attempts` (recorded reason).
    Exhausted { attempts: u32, reason: RetryReason },
    /// Do not retry at all: refusals/robots/local caps — retrying would
    /// hammer an answered question or a deterministic limit.
    NoRetry,
}

/// All knobs of the retry loop. `deterministic` pins jitter to zero for
/// reproducible tests (and any operator who wants exact schedules); the
/// engine default keeps jitter ON because synchronized retry waves from
/// fleets are a real load pattern.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_ms: u64,
    pub cap_ms: u64,
    pub deterministic: bool,
    /// Per-target attempt ceiling (0 = only `max_retries` applies).
    pub per_target_max: u32,
    /// Global retry budget: refilled `budget_per_min` tokens/minute, one
    /// consumed per retry sleep (0 = unlimited). Under a budget storm the
    /// engine fails fast instead of stacking sleeps on a sick upstream.
    pub budget_per_min: u32,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_ms: 500,
            cap_ms: 15_000,
            deterministic: false,
            per_target_max: 0,
            budget_per_min: 0,
        }
    }
}

impl RetryPolicy {
    /// Build from the engine config (keeps fetcher construction sites one
    /// line). `deterministic` here maps `retry_deterministic` in config.
    pub fn from_cfg(cfg: &crate::config::Config) -> Self {
        Self {
            max_retries: cfg.max_retries,
            base_ms: cfg.retry_base_ms,
            cap_ms: cfg.max_single_wait_ms,
            deterministic: cfg.retry_deterministic,
            per_target_max: 0,
            budget_per_min: cfg.scheduling.retry_budget_per_min,
        }
    }

    /// The shipped delay curve, exactly. `jitter_sample` is folded in as
    /// `salt.next() % 251` by the caller so this stays pure.
    pub fn delay_for(&self, attempt: u32, jitter_sample: u64) -> Duration {
        let exp = self
            .base_ms
            .saturating_mul(1u64 << (attempt.saturating_sub(1)).min(5));
        let jitter = if self.deterministic {
            0
        } else {
            jitter_sample % 251
        };
        Duration::from_millis((exp + jitter).min(self.cap_ms))
    }

    /// Pure decision. `already_attempted` counts requests made so far
    /// (starts at 1 after the first try). `is_block`/`is_permanent` encode
    /// the fetcher's classification without this layer re-reading HTTP.
    pub fn decide(
        &self,
        already_attempted: u32,
        msg: &str,
        is_block: bool,
        is_permanent: bool,
        is_robots: bool,
        jitter_sample: u64,
    ) -> RetryDecision {
        if is_block || is_permanent || is_robots {
            return RetryDecision::NoRetry;
        }
        let reason = RetryReason::classify(msg);
        // Deadlines never improve on retry.
        if reason == RetryReason::Deadline {
            return RetryDecision::Exhausted {
                attempts: already_attempted,
                reason,
            };
        }
        if already_attempted > self.max_retries {
            return RetryDecision::Exhausted {
                attempts: already_attempted,
                reason,
            };
        }
        RetryDecision::Retry {
            next_attempt: already_attempted + 1,
            delay: self.delay_for(already_attempted, jitter_sample),
            reason,
        }
    }
}

/// Shared retry budget across everything holding this arc (a run, or all
/// shards' in-flight work when embedded in job state — engine-level today).
/// Token bucket refilled continuously from wall clock; refills never exceed
/// one minute's allotment, so a quiet run cannot bank an infinite burst.
#[derive(Debug)]
pub struct RetryBudget {
    per_min: u32,
    used: AtomicU64,
    window_start_ms: AtomicU64,
    exhausted_events: AtomicUsize,
}

impl RetryBudget {
    /// `per_min == 0` => unlimited (default; no cost to the hot path beyond
    /// one relaxed load per retry).
    pub fn new(per_min: u32) -> Self {
        Self {
            per_min,
            used: AtomicU64::new(0),
            window_start_ms: AtomicU64::new(Self::now_ms()),
            exhausted_events: AtomicUsize::new(0),
        }
    }

    pub fn unlimited(&self) -> bool {
        self.per_min == 0
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn roll_window(&self, now: u64) {
        let start = self.window_start_ms.load(Ordering::Relaxed);
        if now.saturating_sub(start) >= 60_000 {
            // Best-effort single-winner roll; racers just share the window,
            // which only skews fairness by one window, never breaks the cap
            // (a missed roll undercounts *used*, making us stricter).
            if self
                .window_start_ms
                .compare_exchange(start, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                self.used.store(0, Ordering::Relaxed);
            }
        }
    }

    /// Consume one retry token. False = budget spent; caller should stop
    /// retrying (fail-fast) rather than queue more load.
    pub fn try_consume(&self) -> bool {
        if self.unlimited() {
            return true;
        }
        let now = Self::now_ms();
        self.roll_window(now);
        loop {
            let used = self.used.load(Ordering::Relaxed);
            if used >= self.per_min as u64 {
                self.exhausted_events.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            if self
                .used
                .compare_exchange(used, used + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    pub fn snapshot(&self) -> BudgetSnapshot {
        BudgetSnapshot {
            per_min: self.per_min,
            used: self.used.load(Ordering::Relaxed) as usize,
            denied_total: self.exhausted_events.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct BudgetSnapshot {
    pub per_min: u32,
    pub used: usize,
    pub denied_total: usize,
}

/// Per-target attempt accounting (job-level caps beyond the request-level
/// `max_retries`). Engine wires it when `scheduling` limits are configured;
/// otherwise it stays at zero and answers "allowed".
#[derive(Default)]
pub struct AttemptLedger {
    per_target: Vec<AtomicU64>,
}

impl AttemptLedger {
    pub fn new(n_targets: usize) -> Self {
        Self {
            per_target: (0..n_targets).map(|_| AtomicU64::new(0)).collect(),
        }
    }
    pub fn note_attempt(&self, target: usize) {
        if let Some(slot) = self.per_target.get(target) {
            slot.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn attempts(&self, target: usize) -> u64 {
        self.per_target
            .get(target)
            .map(|s| s.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
    pub fn over_limit(&self, target: usize, limit: u32) -> bool {
        limit > 0 && self.attempts(target) >= limit as u64
    }
}

#[cfg(test)]
mod tests;
