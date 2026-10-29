//! Per-key rate limiting, concurrency gating and monthly quota.
//!
//! Three independent limits protect the engine from a single customer:
//!
//! * a **token bucket** per key (sustained rate + burst),
//! * a **concurrency gate** per key (how many of its jobs may be in flight),
//! * a **monthly quota** per key (how many requests it may make this month).
//!
//! All three answer with a stable machine code and a `Retry-After` hint so a
//! client can back off instead of hammering. The limiter is bounded: buckets
//! for keys that have been quiet for `idle_ttl` are evicted, so a key that
//! stopped using the API does not keep the map (and its memory) alive.
//!
//! The clock is injectable. Tests drive refill and expiry deterministically
//! instead of sleeping; production passes the wall clock.
//!
//! See `docs/modules/ratelimit.md`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::tenancy::limits::KeyLimits;

/// Wall-clock hook. Production uses [`RateLimiter::now_ms`]; tests pass a
/// closure that steps a fake clock.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Default idle TTL before a key's bucket is evicted.
pub const DEFAULT_IDLE_TTL_MS: u64 = 5 * 60 * 1000;
/// Default hard cap on simultaneously tracked keys.
pub const DEFAULT_MAX_KEYS: usize = 10_000;

/// Why a request was refused. Each variant carries what the client should
/// send back as `Retry-After`, so the HTTP layer never has to recompute it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitError {
    /// Token bucket empty. `retry_after_secs` is the refill time of one token.
    RateLimited { retry_after_secs: u64 },
    /// Concurrency gate full. `retry_after_secs` is the backoff hint.
    ConcurrencyLimited { retry_after_secs: u64 },
    /// Monthly quota spent. `reset_at` is the epoch second the quota rolls.
    QuotaExhausted { reset_at: i64 },
}

impl RateLimitError {
    pub fn retry_after(&self) -> u64 {
        match self {
            RateLimitError::RateLimited { retry_after_secs }
            | RateLimitError::ConcurrencyLimited { retry_after_secs } => *retry_after_secs,
            RateLimitError::QuotaExhausted { .. } => 1,
        }
    }

    pub fn stable_code(&self) -> &'static str {
        match self {
            RateLimitError::RateLimited { .. } => "rate_limited",
            RateLimitError::ConcurrencyLimited { .. } => "concurrency_limited",
            RateLimitError::QuotaExhausted { .. } => "quota_exhausted",
        }
    }

    pub fn message(&self) -> String {
        match self {
            RateLimitError::RateLimited { retry_after_secs } => {
                format!("request rate limit reached; retry in {retry_after_secs}s")
            }
            RateLimitError::ConcurrencyLimited { retry_after_secs } => {
                format!("concurrency limit reached; retry in {retry_after_secs}s")
            }
            RateLimitError::QuotaExhausted { reset_at } => {
                format!("monthly quota exhausted; resets at epoch {reset_at}")
            }
        }
    }
}

/// RAII permit for a concurrency slot. Dropping it (normally or via panic)
/// returns the slot to the key, so a request that failed mid-flight cannot
/// pin a slot forever.
pub struct Permit<'a> {
    limiter: &'a RateLimiter,
    key: String,
}

impl std::fmt::Debug for Permit<'_> {
    /// Hand-written: the limiter itself is not `Debug`, and the permit only
    /// needs to be printable for `unwrap_err` diagnostics.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Permit")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.limiter.release_concurrency(&self.key);
    }
}

/// One key's bucket state.
#[derive(Debug, Clone)]
struct Bucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last_ms: u64,
    inflight: u32,
    last_activity_ms: u64,
}

/// One key's monthly quota state.
#[derive(Debug, Clone, Copy)]
struct QuotaState {
    month: i64,
    used: u64,
}

/// Current UTC month, expressed as `year * 12 + (month - 1)`.
fn current_month(epoch_secs: i64) -> i64 {
    let days = epoch_secs.div_euclid(86_400);
    let (y, m, _) = crate::ids::civil_from_days(days);
    y * 12 + i64::from(m) - 1
}

/// Epoch second at which the next UTC month begins (the quota reset).
fn next_month_epoch(epoch_secs: i64) -> i64 {
    let days = epoch_secs.div_euclid(86_400);
    let (y, m, _) = crate::ids::civil_from_days(days);
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    days_from_civil(ny, nm, 1) * 86_400
}

/// Unix days since the epoch for a proleptic Gregorian date.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = if yy >= 0 { yy } else { yy - 399 } / 400;
    let yoe = yy - era * 400;
    let mp = if m > 2 {
        i64::from(m) - 3
    } else {
        i64::from(m) + 9
    };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The limiter.
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    quota: Mutex<HashMap<String, QuotaState>>,
    now: Clock,
    max_keys: AtomicUsize,
    idle_ttl_ms: AtomicU64,
}

impl RateLimiter {
    /// System wall clock in milliseconds since the Unix epoch.
    pub fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    pub fn new() -> Self {
        Self::with_clock(Arc::new(Self::now_ms))
    }

    pub fn with_clock(now: Clock) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            quota: Mutex::new(HashMap::new()),
            now,
            max_keys: AtomicUsize::new(DEFAULT_MAX_KEYS),
            idle_ttl_ms: AtomicU64::new(DEFAULT_IDLE_TTL_MS),
        }
    }

    /// Test/replay constructor. Production uses [`RateLimiter::new`].
    pub fn for_tests(now: Clock) -> Self {
        Self::with_clock(now)
    }

    pub fn set_bounds(&self, max_keys: usize, idle_ttl_ms: u64) {
        self.max_keys.store(max_keys.max(1), Ordering::Relaxed);
        self.idle_ttl_ms.store(idle_ttl_ms, Ordering::Relaxed);
    }

    pub fn idle_ttl_ms(&self) -> u64 {
        self.idle_ttl_ms.load(Ordering::Relaxed)
    }

    pub fn max_keys(&self) -> usize {
        self.max_keys.load(Ordering::Relaxed)
    }

    pub fn now(&self) -> u64 {
        (self.now)()
    }

    pub fn tracked_keys(&self) -> usize {
        self.buckets.lock().map(|b| b.len()).unwrap_or(0)
    }

    /// Recompute `Retry-After` from a key's sustained rate. Bounded so a
    /// misconfigured `rps` cannot ask a client to wait a week.
    pub fn retry_after_secs(rps: f64) -> u64 {
        if !rps.is_finite() || rps <= 0.0 {
            return 1;
        }
        (1.0 / rps).ceil().clamp(1.0, 3600.0) as u64
    }

    /// Evict buckets idle for longer than the TTL. Called at the top of every
    /// acquire so the map cannot grow without bound.
    fn evict_idle(&self, buckets: &mut HashMap<String, Bucket>) {
        let now = self.now();
        let ttl = self.idle_ttl_ms();
        // Never evict a bucket that still holds concurrency slots: dropping it
        // would leak the slot and let the key exceed its limit forever.
        buckets.retain(|_, b| b.inflight > 0 || now.saturating_sub(b.last_activity_ms) < ttl);
    }

    /// Acquire a rate-limit token and a concurrency slot for `key`.
    ///
    /// Returns a [`Permit`] that must be dropped (or held for a job's
    /// lifetime) to return the concurrency slot.
    pub fn acquire(&self, key: &str, limits: &KeyLimits) -> Result<Permit<'_>, RateLimitError> {
        if key.is_empty() || key.len() > 128 {
            return Err(RateLimitError::RateLimited {
                retry_after_secs: 1,
            });
        }
        let now = self.now();
        let mut buckets = self.buckets.lock().unwrap_or_else(|p| p.into_inner());
        self.evict_idle(&mut buckets);

        if buckets.len() >= self.max_keys() && !buckets.contains_key(key) {
            // Capacity is full: drop the most idle *idle* bucket (one with no
            // live permits) rather than grow past the bound.
            if let Some(victim) = buckets
                .iter()
                .filter(|(_, b)| b.inflight == 0)
                .min_by_key(|(_, b)| b.last_activity_ms)
                .map(|(k, _)| k.clone())
            {
                buckets.remove(&victim);
            }
        }

        let entry = buckets.entry(key.to_string()).or_insert_with(|| Bucket {
            capacity: f64::from(limits.burst),
            tokens: f64::from(limits.burst),
            refill_per_sec: limits.rps,
            last_ms: now,
            inflight: 0,
            last_activity_ms: now,
        });

        // Refill before the check: a key that waited gets its tokens back.
        let elapsed_ms = now.saturating_sub(entry.last_ms);
        if elapsed_ms > 0 {
            entry.tokens = (entry.tokens + elapsed_ms as f64 / 1000.0 * entry.refill_per_sec)
                .min(entry.capacity);
            entry.last_ms = now;
        }
        entry.last_activity_ms = now;

        if entry.tokens < 1.0 {
            let retry_after = if entry.refill_per_sec.is_finite() && entry.refill_per_sec > 0.0 {
                Self::retry_after_secs(entry.refill_per_sec)
            } else {
                1
            };
            return Err(RateLimitError::RateLimited {
                retry_after_secs: retry_after,
            });
        }
        entry.tokens -= 1.0;

        if entry.inflight >= limits.max_concurrent_jobs {
            return Err(RateLimitError::ConcurrencyLimited {
                retry_after_secs: Self::retry_after_secs(limits.rps),
            });
        }
        entry.inflight += 1;

        drop(buckets);
        Ok(Permit {
            limiter: self,
            key: key.to_string(),
        })
    }

    /// Release a concurrency slot. Called by [`Permit::drop`]; exposed so
    /// long-lived callers (jobs) can release explicitly.
    pub fn release_concurrency(&self, key: &str) {
        if let Ok(mut buckets) = self.buckets.lock() {
            if let Some(b) = buckets.get_mut(key) {
                b.inflight = b.inflight.saturating_sub(1);
            }
        }
    }

    /// Current in-flight count for a key (diagnostics + tests).
    pub fn inflight(&self, key: &str) -> u32 {
        self.buckets
            .lock()
            .ok()
            .and_then(|b| b.get(key).map(|x| x.inflight))
            .unwrap_or(0)
    }

    /// Tokens available now, without consuming (diagnostics + tests).
    pub fn tokens_available(&self, key: &str) -> f64 {
        let now = self.now();
        self.buckets
            .lock()
            .ok()
            .and_then(|mut b| {
                let entry = b.get_mut(key)?;
                let elapsed = now.saturating_sub(entry.last_ms);
                if elapsed > 0 {
                    entry.tokens = (entry.tokens + elapsed as f64 / 1000.0 * entry.refill_per_sec)
                        .min(entry.capacity);
                    entry.last_ms = now;
                }
                Some(entry.tokens)
            })
            .unwrap_or(0.0)
    }

    // ------------------------------------------------------------ quota

    /// Monthly quota check against an observed usage count. Pure hook: the
    /// caller (the usage ledger) supplies how much the key has already spent
    /// this month, so the limiter never becomes the source of truth.
    pub fn check_quota(
        &self,
        used_this_month: u64,
        limits: &KeyLimits,
    ) -> Result<(), RateLimitError> {
        if !limits.quota_enabled() {
            return Ok(());
        }
        if used_this_month >= limits.monthly_quota {
            return Err(RateLimitError::QuotaExhausted {
                reset_at: next_month_epoch(self.now() as i64 / 1000),
            });
        }
        Ok(())
    }

    /// Convenience wrapper used by the handler: read the fast-path counter,
    /// then apply the cap.
    pub fn check_quota_for(&self, key: &str, limits: &KeyLimits) -> Result<(), RateLimitError> {
        self.check_quota(self.used_this_month(key), limits)
    }

    /// Note that `key` consumed `n` requests in the current month. Rolls the
    /// bucket over when the calendar month changes.
    pub fn note_usage(&self, key: &str, n: u64) {
        if n == 0 {
            return;
        }
        let month = current_month(self.now() as i64 / 1000);
        if let Ok(mut quota) = self.quota.lock() {
            let state = quota
                .entry(key.to_string())
                .and_modify(|s| {
                    if s.month != month {
                        *s = QuotaState { month, used: 0 };
                    }
                    s.used = s.used.saturating_add(n);
                })
                .or_insert(QuotaState { month, used: n });
            let _ = state;
        }
    }

    /// How many requests `key` has spent in the current month.
    pub fn used_this_month(&self, key: &str) -> u64 {
        let month = current_month(self.now() as i64 / 1000);
        self.quota
            .lock()
            .ok()
            .and_then(|q| {
                q.get(key)
                    .and_then(|s| (s.month == month).then_some(s.used))
            })
            .unwrap_or(0)
    }

    /// Overwrite the fast-path counter with the ledger's view. Called once at
    /// startup so a restarted process enforces the same quota. A stale month
    /// is ignored rather than written: a late write must never reset a
    /// counter that has already rolled over.
    pub fn seed_used(&self, key: &str, month: i64, used: u64) {
        let current = current_month(self.now() as i64 / 1000);
        if month < current {
            return;
        }
        if let Ok(mut quota) = self.quota.lock() {
            quota.insert(
                key.to_string(),
                QuotaState {
                    month,
                    used: used.min(crate::tenancy::limits::MONTHLY_QUOTA_CEILING),
                },
            );
        }
    }

    pub fn drop_key(&self, key: &str) {
        if let Ok(mut q) = self.quota.lock() {
            q.remove(key);
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Duration helper kept for symmetry with the engine's `Duration`-based
/// APIs. Clamped so a bad configuration cannot ask for a multi-hour wait.
pub fn retry_after_duration(secs: u64) -> Duration {
    Duration::from_secs(secs.min(3600))
}

#[cfg(test)]
mod tests;
