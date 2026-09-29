use super::*;

fn policy() -> RetryPolicy {
    RetryPolicy {
        max_retries: 3,
        base_ms: 500,
        cap_ms: 15_000,
        deterministic: true,
        per_target_max: 0,
        budget_per_min: 0,
    }
}

#[test]
fn delay_curve_matches_legacy_formula_exactly() {
    let p = policy(); // deterministic: jitter term must vanish
                      // legacy: base * 2^min(a-1,5) + jitter%251, min with cap
    for (attempt, expect_ms) in [(1, 500), (2, 1000), (3, 2000), (4, 4000), (7, 16_000)] {
        let d = p.delay_for(attempt, 12_345);
        assert_eq!(
            d,
            Duration::from_millis(expect_ms.min(15_000)),
            "attempt {attempt}"
        );
    }
    let j = RetryPolicy {
        deterministic: false,
        ..policy()
    };
    let lo = j.delay_for(1, 0).as_millis();
    let hi = j.delay_for(1, u64::MAX).as_millis();
    assert!((500..=750).contains(&lo) && (500..=750).contains(&hi));
}

#[test]
fn blocks_and_permanents_are_never_retried() {
    let p = policy();
    for (block, perm, robots) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        assert_eq!(
            p.decide(1, "whatever", block, perm, robots, 0),
            RetryDecision::NoRetry
        );
    }
}

#[test]
fn exhaustion_and_retry_boundaries() {
    let p = policy();
    assert!(matches!(
        p.decide(1, "server status 502", false, false, false, 0),
        RetryDecision::Retry {
            next_attempt: 2,
            reason: RetryReason::Server5xx,
            ..
        }
    ));
    // already_attempted counts requests made; with max_retries=3 the
    // 4th failure (attempted 4 > 3) exhausts — same as the legacy loop.
    assert!(matches!(
        p.decide(4, "timeout elapsed", false, false, false, 0),
        RetryDecision::Exhausted {
            attempts: 4,
            reason: RetryReason::Timeout
        }
    ));
}

#[test]
fn deadline_never_retries() {
    let p = policy();
    assert!(matches!(
        p.decide(1, "deadline exceeded for http://x", false, false, false, 0),
        RetryDecision::Exhausted {
            reason: RetryReason::Deadline,
            ..
        }
    ));
}

#[test]
fn reason_classification_covers_produced_strings() {
    assert_eq!(
        RetryReason::classify("server status 503"),
        RetryReason::Server5xx
    );
    assert_eq!(
        RetryReason::classify("body read: error decoding response body"),
        RetryReason::BodyRead
    );
    assert_eq!(
        RetryReason::classify("error sending request: operation timed out (timeout: 20s)"),
        RetryReason::Timeout
    );
    assert_eq!(
        RetryReason::classify("deadline exceeded for http://x/slow"),
        RetryReason::Deadline
    );
    assert_eq!(
        RetryReason::classify("error trying to connect: Connection refused"),
        RetryReason::Transport
    );
    assert_eq!(
        RetryReason::classify("some bespoke failure"),
        RetryReason::Unclassified
    );
}

#[test]
fn budget_caps_and_denies() {
    let b = RetryBudget::new(3);
    assert!(b.try_consume());
    assert!(b.try_consume());
    assert!(b.try_consume());
    assert!(!b.try_consume(), "4th within the minute must be denied");
    let s = b.snapshot();
    assert_eq!(s.used, 3);
    assert_eq!(s.denied_total, 1);
    let u = RetryBudget::new(0);
    for _ in 0..1000 {
        assert!(u.try_consume(), "unlimited never denies");
    }
}

#[test]
fn attempt_ledger_bounds_and_limit_semantics() {
    let l = AttemptLedger::new(2);
    l.note_attempt(0);
    l.note_attempt(0);
    l.note_attempt(5); // out of range: ignored, never panics
    assert_eq!(l.attempts(0), 2);
    assert!(!l.over_limit(0, 0), "0 means no limit");
    assert!(l.over_limit(0, 2));
    assert!(!l.over_limit(1, 1));
}
