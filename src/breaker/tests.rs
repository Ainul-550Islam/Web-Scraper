use super::*;

fn breaker(min: u32, cooldown_ms: u64) -> Breaker {
    Breaker::new(BreakerCfg {
        min_consecutive: min,
        cooldown: Duration::from_millis(cooldown_ms),
        recovery_successes: 1,
    })
}

#[test]
fn stays_closed_below_threshold_opens_at_it() {
    let b = breaker(3, 100);
    assert!(b.note_failure(Instant::now()).is_none());
    assert!(b.note_failure(Instant::now()).is_none());
    assert_eq!(b.state(), BreakerState::Closed);
    assert_eq!(
        b.note_failure(Instant::now()),
        Some(BreakerEvent::ClosedToOpen)
    );
    assert_eq!(b.state(), BreakerState::Open);
}

#[test]
fn success_resets_consecutive_count() {
    let b = breaker(3, 100);
    b.note_failure(Instant::now());
    b.note_failure(Instant::now());
    b.note_success(Instant::now());
    b.note_failure(Instant::now());
    b.note_failure(Instant::now());
    assert_eq!(b.state(), BreakerState::Closed, "2+2 straddling success");
    b.note_failure(Instant::now());
    assert_eq!(b.state(), BreakerState::Open);
}

#[test]
fn open_rejects_then_half_open_admits_exactly_one_probe() {
    let b = breaker(2, 50);
    b.note_failure(Instant::now());
    b.note_failure(Instant::now());
    let now = Instant::now();
    assert!(matches!(b.permit(now), Permit::Open { .. }));
    // Cooldown elapses -> first caller becomes the probe, second is
    // still rejected while it is in flight.
    std::thread::sleep(Duration::from_millis(60));
    assert_eq!(b.permit(Instant::now()), Permit::Probe);
    assert!(matches!(b.permit(Instant::now()), Permit::Open { .. }));
    let snap = b.snapshot();
    assert_eq!(snap.state, BreakerState::HalfOpen);
    assert_eq!(snap.transitions, 2, "opened, then ->half-open");
}

#[test]
fn probe_success_closes_probe_failure_reopens() {
    let b = breaker(1, 0);
    b.note_failure(Instant::now()); // opens immediately (min=1, cooldown 0)
    assert_eq!(b.permit(Instant::now()), Permit::Probe);
    assert_eq!(
        b.note_success(Instant::now()),
        Some(BreakerEvent::HalfOpenToClosed)
    );
    assert_eq!(b.state(), BreakerState::Closed);
    assert_eq!(b.snapshot().total_opens, 1);

    // reopen and fail the probe
    b.note_failure(Instant::now());
    assert_eq!(b.permit(Instant::now()), Permit::Probe);
    assert_eq!(
        b.note_failure(Instant::now()),
        Some(BreakerEvent::HalfOpenToOpen),
        "failed probe must reopen"
    );
    assert_eq!(b.state(), BreakerState::Open);
}

#[test]
fn recovery_needs_configured_successes() {
    let b = Breaker::new(BreakerCfg {
        min_consecutive: 1,
        cooldown: Duration::ZERO,
        recovery_successes: 3,
    });
    b.note_failure(Instant::now());
    assert_eq!(b.permit(Instant::now()), Permit::Probe);
    assert!(b.note_success(Instant::now()).is_none(), "1/3");
    assert!(b.note_success(Instant::now()).is_none(), "2/3");
    assert_eq!(
        b.note_success(Instant::now()),
        Some(BreakerEvent::HalfOpenToClosed)
    );
    assert_eq!(b.state(), BreakerState::Closed);
}

#[test]
fn reset_clears_state_but_keeps_counters() {
    let b = breaker(1, 10_000);
    b.note_failure(Instant::now());
    assert_eq!(b.state(), BreakerState::Open);
    b.reset();
    assert_eq!(b.state(), BreakerState::Closed);
    assert!(matches!(b.permit(Instant::now()), Permit::Allowed));
    assert_eq!(b.snapshot().transitions, 1, "history survives reset");
}

#[test]
fn open_snapshot_exposes_how_long_open() {
    let b = breaker(1, 1000);
    b.note_failure(Instant::now());
    std::thread::sleep(Duration::from_millis(15));
    let s = b.snapshot();
    assert_eq!(s.state, BreakerState::Open);
    assert!(s.opened_ms_ago >= 10, "{s:?}");
}
