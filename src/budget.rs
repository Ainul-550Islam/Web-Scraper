//! Bounded resource budgets — the shared primitive behind every "fail
//! safely at a limit" control that isn't already owned elsewhere.
//!
//! Deliberate scope (the architecture audit's answer to "don't duplicate
//! budgets"): the scheduler already owns queue depth and per-target
//! inflight admission ([`crate::sched`]), retries use the sliding-window
//! [`crate::retry::RetryBudget`], transport refusals use
//! [`crate::breaker::CircuitBreaker`], and response bytes are capped at the
//! fetcher (`max_response_bytes`). This module exists for the limits those
//! do not cover — proxy in-flight holds and run-scoped consumption caps —
//! and defines the one acquire/release protocol they all follow, so the
//! semantics (never block, never steal, count the denial) stay uniform.
//!
//! Invariants every budget upholds:
//!
//! * `limit == 0` means **unlimited**, and is the config default: behavior
//!   is byte-identical to a build without budgets.
//! * A denied acquisition changes nothing (no half-taken state).
//! * Denials are counted, never silent: every budget reports `denied`,
//!   and callers surface it (events, metrics, or a returned error).
//! * Holds return their quota on drop — a panic between acquire and drop
//!   cannot leak capacity (the leak test pins this).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// A named, bounded quota. `name` is a fixed label used in diagnostics and
/// metrics — budget names are compile-time constants, never user input
/// (high-cardinality discipline, same rule as metrics labels).
#[derive(Debug)]
pub struct Budget {
    name: &'static str,
    limit: u64,
    used: AtomicU64,
    denied: AtomicU64,
    peak: AtomicU64,
}

impl Budget {
    pub fn new(name: &'static str, limit: u64) -> Self {
        Self {
            name,
            limit,
            used: AtomicU64::new(0),
            denied: AtomicU64::new(0),
            peak: AtomicU64::new(0),
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
    /// 0 = unlimited.
    pub fn limit(&self) -> u64 {
        self.limit
    }
    pub fn is_limited(&self) -> bool {
        self.limit > 0
    }
    pub fn used(&self) -> u64 {
        self.used.load(Ordering::Relaxed)
    }
    pub fn denied(&self) -> u64 {
        self.denied.load(Ordering::Relaxed)
    }
    pub fn peak(&self) -> u64 {
        self.peak.load(Ordering::Relaxed)
    }

    /// Monotonic consumption cap: take `n` or fail (no release — for
    /// per-run totals like records written). Unlimited budgets always take.
    pub fn try_take(&self, n: u64) -> bool {
        if self.limit == 0 {
            self.used.fetch_add(n, Ordering::Relaxed);
            self.recompute_peak();
            return true;
        }
        // CAS loop: add only if the total stays within the limit.
        let mut cur = self.used.load(Ordering::Relaxed);
        loop {
            let next = match cur.checked_add(n) {
                Some(v) if v <= self.limit => v,
                _ => {
                    self.denied.fetch_add(1, Ordering::Relaxed);
                    return false;
                }
            };
            match self
                .used
                .compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.track_peak(next);
                    return true;
                }
                Err(obs) => cur = obs,
            }
        }
    }

    /// Acquire-with-release (for in-flight style limits). Returns `None` on
    /// denial (counted). The returned [`BudgetHold`] gives the quota back
    /// when dropped — including on a panic path.
    pub fn try_hold(self: &Arc<Self>, n: u64) -> Option<BudgetHold> {
        // Take FIRST: constructing the holder before a denied take would
        // drop it and release capacity it never held (the unit test pins
        // exactly that race).
        if !self.try_take(n) {
            return None;
        }
        Some(BudgetHold {
            _inner: Arc::new(HoldInner {
                budget: self.clone(),
                n,
            }),
        })
    }

    /// Unbounded release path for external holders that counted via
    /// [`Budget::take_unbounded`] (the proxy pool pattern: pick always
    /// counts, the cap only steers selection, so nothing is ever "denied"
    /// by `used` itself).
    pub fn release(&self, n: u64) {
        self.used.fetch_sub(n, Ordering::Relaxed);
    }

    fn recompute_peak(&self) {
        let u = self.used.load(Ordering::Relaxed);
        self.track_peak(u);
    }
    fn track_peak(&self, used: u64) {
        let mut p = self.peak.load(Ordering::Relaxed);
        while used > p {
            match self
                .peak
                .compare_exchange_weak(p, used, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(obs) => p = obs,
            }
        }
    }

    /// Snapshot for diagnostics/JSON: bounded, fixed shape.
    pub fn view(&self) -> BudgetView {
        BudgetView {
            name: self.name,
            limit: self.limit,
            used: self.used(),
            denied: self.denied(),
            peak: self.peak(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BudgetView {
    pub name: &'static str,
    pub limit: u64,
    pub used: u64,
    pub denied: u64,
    pub peak: u64,
}

struct HoldInner {
    budget: Arc<Budget>,
    n: u64,
}

impl Drop for HoldInner {
    fn drop(&mut self) {
        self.budget.release(self.n);
    }
}

/// RAII quota holder; releasing is automatic and panic-safe.
pub struct BudgetHold {
    // Drop is the only reader; the field exists to own the slot.
    _inner: Arc<HoldInner>,
}

// ------------------------------------------------------------------ config

/// The budget-shaped subset of the config, resolved once per run so engine,
/// export, and dist see identical effective limits (typed defaults, `0` =
/// unlimited).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BudgetCfg {
    /// Simultaneous in-flight requests through one proxy egress.
    pub max_inflight_per_proxy: u64,
    /// Total records a single run may write; the run drains gracefully at
    /// the cap (no truncation mid-record, no error — a capped harvest that
    /// still checkpoints, audits, and exits 0).
    pub max_records_per_run: u64,
    /// Target bytes per exported payload part; exceeding it starts a new
    /// part (`export` chunking). 0 = single part (legacy behavior).
    pub max_export_part_bytes: u64,
}

#[cfg(test)]
mod tests;
