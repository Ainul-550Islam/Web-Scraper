use super::*;

#[test]
fn policy_mirrors_config_exactly() {
    let cfg = Config {
        max_retries: 2,
        retry_base_ms: 100,
        max_single_wait_ms: 900,
        retry_deterministic: true,
        ..Default::default()
    };
    let f = Fetcher::new(Arc::new(cfg.clone()), Arc::new(ProxyPool::new(&[])));
    assert_eq!(f.policy.max_retries, 2);
    assert_eq!(f.policy.base_ms, 100);
    assert_eq!(f.policy.cap_ms, 900);
    assert!(f.policy.deterministic);
    // legacy formula at attempt 1: 100*1 + jitter, capped — jitter 0 =>
    // exact doubling curve, cap enforced:
    assert_eq!(f.policy.delay_for(1, 250), Duration::from_millis(100));
    assert_eq!(f.policy.delay_for(4, 250), Duration::from_millis(800));
    assert_eq!(f.policy.delay_for(5, 250), Duration::from_millis(900));
}

#[test]
fn retry_after_seconds() {
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(reqwest::header::RETRY_AFTER, "120".parse().unwrap());
    assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(120)));
    h.insert(reqwest::header::RETRY_AFTER, "999999".parse().unwrap());
    assert_eq!(parse_retry_after(&h), Some(Duration::from_secs(3600)));
}

#[test]
fn retry_after_http_date_future() {
    // A clearly-future fixed date (no tz dependence: we compute from epoch).
    let mut h = reqwest::header::HeaderMap::new();
    h.insert(
        reqwest::header::RETRY_AFTER,
        "Sat, 01 Jan 2044 00:05:00 GMT".parse().unwrap(),
    );
    let d = parse_retry_after(&h).expect("parsed");
    assert!(
        d > Duration::from_secs(300) && d <= Duration::from_secs(3600),
        "{d:?}"
    );
}

#[test]
fn retry_after_absent_or_garbage() {
    let h = reqwest::header::HeaderMap::new();
    assert_eq!(parse_retry_after(&h), None);
    let mut h2 = reqwest::header::HeaderMap::new();
    h2.insert(reqwest::header::RETRY_AFTER, "soon maybe".parse().unwrap());
    assert_eq!(parse_retry_after(&h2), None);
}

#[test]
fn block_statuses_classified() {
    assert!(is_block(401) && is_block(403) && is_block(429) && is_block(451));
    assert!(!is_block(404) && !is_block(500) && !is_block(200));
}
