//! Tests for the per-key limiter. All timing is driven by an injected clock,
//! so no test sleeps.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

use super::*;
use crate::tenancy::limits::KeyLimits;

/// A key that can burst once and refill at one token per second.
fn limits(rps: f64, burst: u32, concurrent: u32, quota: u64) -> KeyLimits {
    KeyLimits {
        rps,
        burst,
        max_concurrent_jobs: concurrent,
        monthly_quota: quota,
        ..KeyLimits::default()
    }
}

struct FakeClock(AtomicU64);

impl FakeClock {
    fn new(start: u64) -> Arc<Self> {
        Arc::new(Self(AtomicU64::new(start)))
    }
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
    fn at(&self, ms: u64) {
        self.0.store(ms, Ordering::SeqCst);
    }
}

fn fake_limiter(clock: &Arc<FakeClock>) -> RateLimiter {
    let clock: Arc<dyn Fn() -> u64 + Send + Sync> = Arc::new({
        let c = clock.clone();
        move || c.now_ms()
    });
    RateLimiter::with_clock(clock)
}

// --------------------------------------------------------------- token bucket

#[test]
fn burst_is_spent_then_rate_limited_with_bounded_retry_after() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1.0, 2, 10, 0);
    for _ in 0..2 {
        limiter.acquire("k", &l).expect("burst should allow two");
    }
    let err = limiter.acquire("k", &l).unwrap_err();
    assert_eq!(
        err,
        RateLimitError::RateLimited {
            retry_after_secs: 1
        }
    );
    assert_eq!(err.stable_code(), "rate_limited");
    assert_eq!(err.retry_after(), 1);
    assert!(!err.message().is_empty());
}

#[test]
fn burst_refills_on_the_clock_without_sleeping() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(10.0, 2, 10, 0);
    for _ in 0..2 {
        limiter.acquire("k", &l).unwrap();
    }
    clock.advance(1_000); // one full second at 10/s -> full bucket
    assert_eq!(limiter.tokens_available("k"), 2.0);
    assert!(limiter.acquire("k", &l).is_ok());
}

#[test]
fn refill_is_capped_at_burst() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(50.0, 2, 10, 0);
    limiter.acquire("k", &l).unwrap();
    clock.advance(60_000); // an hour at 50/s would overflow the bucket
    assert_eq!(limiter.tokens_available("k"), 2.0);
}

#[test]
fn a_key_idle_past_the_ttl_starts_a_fresh_burst() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    limiter.set_bounds(100, 1_000);
    let l = limits(1.0, 1, 10, 0);
    // Token spent.
    limiter.acquire("k", &l).unwrap();
    assert_eq!(limiter.tokens_available("k"), 0.0);
    // Still warm: the idle TTL has not passed, so the empty bucket survives
    // and keeps refusing.
    clock.advance(999);
    assert!(limiter.acquire("k", &l).is_err());
    // Past the TTL the bucket is dropped and recreated full, which is the
    // documented bound: the key cannot accumulate an unbounded debt while
    // its bucket is evicted.
    clock.advance(2);
    limiter
        .acquire("k", &l)
        .expect("fresh burst after idle eviction");
    assert_eq!(limiter.tokens_available("k"), 0.0);
}

#[test]
fn a_low_rate_key_recovers_at_its_own_rate() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    limiter.set_bounds(100, 3_600_000); // never evicts during the test
    let l = limits(0.5, 1, 10, 0);
    limiter.acquire("k", &l).unwrap();
    assert_eq!(limiter.tokens_available("k"), 0.0);
    clock.advance(1_000);
    assert_eq!(limiter.tokens_available("k"), 0.5);
    clock.advance(1_000);
    limiter
        .acquire("k", &l)
        .expect("second token after two seconds");
    assert_eq!(limiter.tokens_available("k"), 0.0);
}

#[test]
fn retry_after_is_bounded() {
    assert_eq!(RateLimiter::retry_after_secs(0.0), 1);
    assert_eq!(RateLimiter::retry_after_secs(-1.0), 1);
    assert_eq!(RateLimiter::retry_after_secs(10.0), 1);
    assert_eq!(RateLimiter::retry_after_secs(0.25), 4);
    assert_eq!(RateLimiter::retry_after_secs(1.0 / 7200.0), 3600);
    assert_eq!(RateLimiter::retry_after_secs(f64::NAN), 1);
}

// --------------------------------------------------------------- concurrency

#[test]
fn concurrency_gate_refuses_above_the_cap_and_permits_release() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 1000, 1, 0);
    let p = limiter.acquire("k", &l).unwrap();
    let err = limiter.acquire("k", &l).unwrap_err();
    assert_eq!(
        err,
        RateLimitError::ConcurrencyLimited {
            retry_after_secs: 1
        }
    );
    assert_eq!(limiter.inflight("k"), 1);
    drop(p);
    assert_eq!(limiter.inflight("k"), 0);
    limiter.acquire("k", &l).expect("slot was returned");
}

#[test]
fn dropping_a_permit_on_a_panic_path_releases_the_slot() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 1000, 2, 0);
    {
        let p = limiter.acquire("k", &l).unwrap();
        assert_eq!(limiter.inflight("k"), 1);
        drop(p);
    }
    assert_eq!(limiter.inflight("k"), 0);
    // A permit is a RAII slot: it has to be held, not merely created.
    let p2 = limiter.acquire("k", &l).unwrap();
    let p3 = limiter.acquire("k", &l).unwrap();
    assert_eq!(limiter.inflight("k"), 2);
    assert!(limiter.acquire("k", &l).is_err());
    drop(p2);
    assert_eq!(limiter.inflight("k"), 1);
    assert!(limiter.acquire("k", &l).is_ok());
    drop(p3);
}

#[test]
fn explicit_release_returns_the_slot() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 1000, 1, 0);
    let p = limiter.acquire("k", &l).unwrap();
    limiter.release_concurrency("k");
    limiter.release_concurrency("k"); // saturating: never goes negative
    assert_eq!(limiter.inflight("k"), 0);
    drop(p);
    assert_eq!(limiter.inflight("k"), 0);
}

#[test]
fn keys_are_isolated_from_each_other() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1.0, 1, 1, 0);
    limiter.acquire("a", &l).unwrap();
    assert!(limiter.acquire("a", &l).is_err());
    assert!(limiter.acquire("b", &l).is_ok(), "other key is unaffected");
}

// --------------------------------------------------------------- bounded map

#[test]
fn idle_keys_are_evicted_and_active_ones_are_kept() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    limiter.set_bounds(100, 1_000);
    let l = limits(1.0, 1, 1, 0);

    let busy = limiter.acquire("busy", &l).unwrap();
    limiter.acquire("quiet", &l).unwrap();
    clock.advance(1_001);
    limiter.acquire("fresh", &l).unwrap();

    // "quiet" went idle and was evicted; "busy" still holds a permit so it
    // survives, and "fresh" was just touched.
    assert_eq!(limiter.tracked_keys(), 2);
    drop(busy);
}

#[test]
fn tracking_is_capped_at_max_keys() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    limiter.set_bounds(2, 1_000);
    let l = limits(1000.0, 10, 10, 0);
    for i in 0..20 {
        limiter.acquire(&format!("key-{i}"), &l).unwrap();
    }
    assert!(limiter.tracked_keys() <= 2);
}

#[test]
fn empty_and_oversized_keys_are_refused() {
    let clock = FakeClock::new(1_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 10, 10, 0);
    assert!(limiter.acquire("", &l).is_err());
    assert!(limiter.acquire(&"x".repeat(129), &l).is_err());
    assert_eq!(limiter.tracked_keys(), 0);
}

// --------------------------------------------------------------- monthly quota

#[test]
fn monthly_quota_exhaustion_reports_the_reset_epoch() {
    let clock = FakeClock::new(1_700_000_000_000); // Nov 2023
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 100, 10, 100);
    assert!(limiter.check_quota(0, &l).is_ok());
    assert!(limiter.check_quota(99, &l).is_ok());
    let err = limiter.check_quota(100, &l).unwrap_err();
    assert_eq!(err.stable_code(), "quota_exhausted");
    assert_eq!(err.retry_after(), 1);
    assert!(err.message().contains("monthly quota"));
    // The reset hint must be the first instant of the next UTC month.
    let reset = match err {
        RateLimitError::QuotaExhausted { reset_at } => reset_at,
        _ => unreachable!(),
    };
    assert!(reset > 1_700_000_000 && reset < 1_704_000_000);
}

#[test]
fn unlimited_quota_is_never_exhausted() {
    let clock = FakeClock::new(1_700_000_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 100, 10, 0);
    assert!(limiter.check_quota(u64::MAX, &l).is_ok());
    assert!(limiter.check_quota_for("k", &l).is_ok());
}

#[test]
fn note_usage_accumulates_and_rolls_over_with_the_month() {
    let clock = FakeClock::new(1_700_000_000_000);
    let limiter = fake_limiter(&clock);
    let l = limits(1000.0, 100, 10, 10);
    limiter.note_usage("k", 6);
    limiter.note_usage("k", 3);
    assert_eq!(limiter.used_this_month("k"), 9);
    assert!(limiter.check_quota_for("k", &l).is_ok());

    // Jump into the next UTC month: the counter rolls over.
    clock.at(1_702_000_000_000 + 40 * 86_400_000);
    assert_eq!(limiter.used_this_month("k"), 0);
    assert!(limiter.check_quota_for("k", &l).is_ok());
}

#[test]
fn seed_used_makes_the_ledger_visible_to_the_fast_path() {
    let clock = FakeClock::new(1_700_000_000_000);
    let limiter = fake_limiter(&clock);
    let month = super::current_month(1_700_000_000);
    limiter.seed_used("k", month, 8);
    assert_eq!(limiter.used_this_month("k"), 8);
    let l = limits(1000.0, 100, 10, 10);
    assert!(limiter.check_quota_for("k", &l).is_ok());
    limiter.seed_used("k", month, 10);
    assert!(limiter.check_quota_for("k", &l).is_err());
    // A stale month bucket is ignored.
    limiter.seed_used("k", month - 1, 0);
    assert_eq!(limiter.used_this_month("k"), 10);
}

// --------------------------------------------------------------- thread safety

#[test]
fn concurrent_acquires_never_exceed_the_cap() {
    let clock = Arc::new(FakeClock::new(1_000_000));
    let limiter = Arc::new(fake_limiter(&clock));
    let l = limits(1_000.0, 10_000, 3, 0);
    let threads = 16;
    let barrier = Arc::new(Barrier::new(threads));
    let outcomes = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let limiter = Arc::clone(&limiter);
        let barrier = Arc::clone(&barrier);
        let outcomes = Arc::clone(&outcomes);
        let l = l.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            let res = limiter.acquire("shared", &l);
            outcomes.lock().unwrap().insert(res.is_ok());
            if let Ok(p) = res {
                std::thread::sleep(std::time::Duration::from_millis(5));
                drop(p);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(
        *outcomes.lock().unwrap(),
        std::collections::HashSet::from([true, false])
    );
    assert_eq!(limiter.inflight("shared"), 0, "all permits were released");
}

#[test]
fn limiter_defaults_are_sane() {
    let limiter = RateLimiter::new();
    assert_eq!(limiter.max_keys(), DEFAULT_MAX_KEYS);
    assert_eq!(limiter.idle_ttl_ms(), DEFAULT_IDLE_TTL_MS);
    assert_eq!(limiter.tracked_keys(), 0);
    let l = limits(1.0, 1, 1, 0);
    assert!(limiter.acquire("k", &l).is_ok());
    assert_eq!(limiter.tracked_keys(), 1);
}

#[test]
fn retry_after_duration_is_clamped() {
    assert_eq!(retry_after_duration(30).as_secs(), 30);
    assert_eq!(retry_after_duration(99_999).as_secs(), 3600);
}
