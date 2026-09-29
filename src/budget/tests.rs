use super::*;

#[test]
fn unlimited_budget_never_denies() {
    let b = Budget::new("t_unlim", 0);
    for _ in 0..1000 {
        assert!(b.try_take(1_000_000));
    }
    assert_eq!(b.denied(), 0);
    assert!(b.used() >= 1_000_000, "usage still tracks for visibility");
    assert_eq!(b.view().name, "t_unlim");
}

#[test]
fn bounded_take_stops_exactly_at_the_limit() {
    let b = Budget::new("t_cap", 10);
    assert!(b.try_take(4));
    assert!(b.try_take(6));
    assert_eq!(b.used(), 10);
    assert!(!b.try_take(1), "no headroom left");
    assert_eq!(b.denied(), 1, "the denial was counted");
    assert_eq!(b.peak(), 10);
    // overflow-safe: a request larger than the limit denies instead of
    // wrapping (checked_add boundary).
    assert!(!b.try_take(u64::MAX));
    assert_eq!(b.used(), 10, "denied acquisitions change nothing");
}

#[test]
fn holds_release_on_drop_and_never_leak() {
    let b = Arc::new(Budget::new("t_hold", 2));
    let h1 = b.try_hold(1).unwrap();
    let h2 = b.try_hold(1).unwrap();
    assert!(b.try_hold(1).is_none(), "full");
    drop(h1);
    let h3 = b.try_hold(1).expect("released capacity is acquirable");
    assert!(b.try_hold(1).is_none());
    drop(h2);
    drop(h3);
    assert_eq!(b.used(), 0, "every hold gave its quota back");
    // a panicking holder still releases (Drop runs during unwind)
    let h = b.try_hold(1).unwrap();
    let b2 = b.clone();
    let _ = std::panic::catch_unwind(move || {
        let _g = h;
        panic!("mid-use panic");
    });
    assert_eq!(b2.used(), 0, "quota must not leak across a panic");
}

#[test]
fn concurrent_contention_never_oversubscribes() {
    // 8 threads racing for a budget of 100 single units: the sum taken
    // must be exactly 100, the rest counted as denials.
    let b = Arc::new(Budget::new("t_race", 100));
    let mut ts = Vec::new();
    for _ in 0..8 {
        let b = b.clone();
        ts.push(std::thread::spawn(move || {
            let mut won = 0u64;
            for _ in 0..50 {
                if b.try_take(1) {
                    won += 1;
                }
            }
            won
        }));
    }
    let won: u64 = ts.into_iter().map(|t| t.join().unwrap()).sum();
    assert_eq!(won, 100, "exactly the limit was handed out");
    assert_eq!(b.used(), 100);
    assert_eq!(b.denied(), 8 * 50 - 100);
}
