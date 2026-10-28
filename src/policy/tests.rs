use super::*;

fn cfg(rps: f64, min_ms: u64) -> Config {
    Config {
        default_policy: crate::config::DomainPolicy {
            requests_per_sec: rps,
            burst: 1,
            min_interval_ms: min_ms,
        },
        ..Config::default()
    }
}

#[tokio::test]
async fn token_bucket_enforces_min_interval() {
    let c = cfg(100.0, 120);
    let p = Arc::new(Politeness::new(&c));
    let m = Metrics::default();
    let t0 = Instant::now();
    p.wait("h", &c, &m).await;
    p.wait("h", &c, &m).await;
    p.wait("h", &c, &m).await;
    let elapsed = t0.elapsed();
    // 3 calls with min-interval 120ms: at least ~240ms of spacing.
    assert!(
        elapsed >= Duration::from_millis(220),
        "rate limiting not enforced: {elapsed:?}"
    );
}

#[test]
fn backoff_curve_is_exact_and_capped() {
    // Default policy: block_backoff_secs=60, cap 3600, breaker at 5.
    // Pinning the exact curve: dashboards/alerting key off these numbers.
    let c = cfg(1.0, 0);
    let p = Politeness::new(&c);
    for want in [60u64, 120, 240, 480] {
        assert_eq!(p.note_block("h", &c, None), Duration::from_secs(want));
    }
    assert!(!p.is_abandoned("h", &c), "trip happens ON the 5th block");
    assert_eq!(p.note_block("h", &c, None), Duration::from_secs(960));
    assert!(p.is_abandoned("h", &c));
    // continues doubling to the 1h cap, never past it
    assert_eq!(p.note_block("h", &c, None), Duration::from_secs(1920));
    assert_eq!(p.note_block("h", &c, None), Duration::from_secs(3600));
    assert_eq!(p.note_block("h", &c, None), Duration::from_secs(3600));
}

#[test]
fn retry_after_is_respected_within_the_band() {
    let c = cfg(1.0, 0);
    let p = Politeness::new(&c);
    // first block: computed backoff 60s; a SHORTER Retry-After must not
    // undercut our own politeness, a LONGER one is honored, but a
    // long header refuses the host; it must never shorten a site restriction.
    assert_eq!(
        p.note_block("a", &c, Some(Duration::from_secs(5))),
        Duration::from_secs(60)
    );
    let p = Politeness::new(&c);
    assert_eq!(
        p.note_block("b", &c, Some(Duration::from_secs(90))),
        Duration::from_secs(90)
    );
    let p = Politeness::new(&c);
    assert_eq!(
        p.note_block("c", &c, Some(Duration::from_secs(7200))),
        Duration::from_secs(7200)
    );
    assert!(p.is_abandoned("c", &c));
}

#[test]
fn success_resets_backoff_but_not_abandonment() {
    let c = cfg(1.0, 0);
    let p = Politeness::new(&c);
    p.note_block("h", &c, None);
    p.note_block("h", &c, None);
    p.note_success("h", &c);
    assert_eq!(
        p.note_block("h", &c, None),
        Duration::from_secs(60),
        "a clean page resets the doubling"
    );
    // burn through the breaker: abandonment is sticky FOR THE RUN — an
    // operator decision (config change), not something a lucky 200 undoes
    for _ in 0..4 {
        p.note_block("h", &c, None);
    }
    assert!(p.is_abandoned("h", &c));
    p.note_success("h", &c);
    assert!(p.is_abandoned("h", &c), "abandonment must be sticky in-run");
    // and it is per-host: neighbors keep going
    assert!(!p.is_abandoned("other.com", &c));
}

#[test]
fn block_backoff_doubles_and_breaker_trips() {
    let c = cfg(1.0, 0);
    let p = Politeness::new(&c);
    // breaker_count default 5
    let mut last = Duration::ZERO;
    for i in 0..5 {
        let d = p.note_block("h", &c, None);
        assert!(d >= last, "backoff must not shrink");
        last = d;
        let _ = i;
    }
    assert!(p.is_abandoned("h", &c), "breaker should have tripped");
}

#[test]
fn retry_after_is_honored_as_floor() {
    let c = cfg(1.0, 0);
    let p = Politeness::new(&c);
    let d = p.note_block("h", &c, Some(Duration::from_secs(600)));
    assert!(
        d >= Duration::from_secs(600),
        "Retry-After floor ignored: {d:?}"
    );
}
