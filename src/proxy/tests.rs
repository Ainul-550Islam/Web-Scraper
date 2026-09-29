use super::*;

fn cfgs(ids: &[&str]) -> Vec<ProxyCfg> {
    ids.iter()
        .map(|i| ProxyCfg {
            id: i.to_string(),
            url: None,
            region: None,
        })
        .collect()
}

#[test]
fn empty_config_yields_single_direct_entry() {
    let p = ProxyPool::new(&[]);
    assert_eq!(p.snapshot().len(), 1);
    assert_eq!(p.id(0), "direct");
    assert!(p.proxy_url(0).unwrap().is_none());
}

#[test]
fn drained_proxy_gets_fewer_picks() {
    let p = ProxyPool::new(&cfgs(&["good", "bad"]));
    for _ in 0..20 {
        p.report(1, false, 5000); // "bad" is index 1 after sort
    }
    let mut counts = [0u32; 2];
    for s in 0..100u64 {
        counts[p.pick(s * 7919 + 13)] += 1;
    }
    assert!(
        counts[0] > counts[1] * 4,
        "health scoring not effective: {counts:?}"
    );
}

#[test]
fn ewma_update_is_exact_formula() {
    // alpha=0.2, start latency 50ms/success 1.0: one 150ms success must
    // move to EXACTLY 0.8*50+0.2*150 = 70 (pinned: a silent alpha change
    // would break operator dashboards comparing across versions).
    let p = ProxyPool::new(&cfgs(&["only"]));
    p.report(0, true, 150);
    let s = &p.snapshot()[0];
    assert_eq!(s.latency_ewma_ms, 70.0);
    assert_eq!(s.success_ewma, 1.0);
    assert_eq!(s.requests, 1);
    assert_eq!(s.failures, 0);
    // one failure after: success = 0.8*1.0 + 0.2*0 = 0.8
    p.report(0, false, 30);
    let s = &p.snapshot()[0];
    assert_eq!(s.success_ewma, 0.8);
    assert_eq!(s.latency_ewma_ms, (1.0 - 0.2) * 70.0 + 0.2 * 30.0);
    assert_eq!(s.failures, 1);
}

#[test]
fn report_out_of_range_and_url_lookup_are_total_not_panics() {
    let p = ProxyPool::new(&cfgs(&["a", "b"]));
    p.report(99, false, 10); // engine never indexes past the pool; be nice anyway
    assert_eq!(p.snapshot()[0].requests + p.snapshot()[1].requests, 0);
    assert!(p.proxy_url(1234).is_none(), "out-of-range idx returns None");
    assert_eq!(
        p.id(1234),
        "direct",
        "unknown id falls back to label, never panic"
    );
}

#[test]
fn picks_are_deterministic_for_equal_pools() {
    // Same config => same pick for same salt, across processes (the
    // `pick(salt)` contract the engine relies on for reproducibility).
    let a = ProxyPool::new(&cfgs(&["x", "y", "z"]));
    let b = ProxyPool::new(&cfgs(&["z", "y", "x"])); // construction order irrelevant (sorted)
    for s in 0..64u64 {
        assert_eq!(a.pick(s * 31 + 7), b.pick(s * 31 + 7), "salt {s}");
    }
}

#[test]
fn weights_are_bounded_no_panic_on_zero() {
    let p = ProxyPool::new(&cfgs(&["a"]));
    for s in [0u64, 1, u64::MAX] {
        assert_eq!(p.pick(s), 0);
    }
}
