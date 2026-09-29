use super::*;

#[test]
fn ua_token_extraction() {
    assert_eq!(ua_token("harvest/0.1 (data engine)"), "harvest");
    assert_eq!(ua_token("MyBot"), "mybot");
    assert_eq!(ua_token(""), "harvest");
}

#[test]
fn dedup_key_ignores_fragment_keeps_query() {
    let a = Url::parse("http://x.com/p/1?q=2#frag").unwrap();
    let b = Url::parse("http://x.com/p/1?q=2").unwrap();
    assert_eq!(dedup_key(&a), dedup_key(&b));
    let c = Url::parse("http://x.com/p/1?q=3").unwrap();
    assert_ne!(dedup_key(&a), dedup_key(&c));
}

#[test]
fn origin_carries_port_and_auth_out_of_the_url() {
    // robots cache + politeness are keyed per origin; port must NOT be
    // dropped or two services on one host share one robots.txt.
    let u = Url::parse("http://127.0.0.1:8099/a/b?x=1#y").unwrap();
    assert_eq!(origin(&u).as_str(), "http://127.0.0.1:8099/");
    let v = Url::parse("https://ex.com/path").unwrap();
    assert_eq!(origin(&v).as_str(), "https://ex.com/");
    assert_ne!(
        origin(&Url::parse("http://h:1/").unwrap()),
        origin(&Url::parse("http://h:2/").unwrap())
    );
}

#[test]
fn mk_job_applies_age_policy_at_the_single_construction_point() {
    let u = Url::parse("http://x/").unwrap();
    let cfg = Config::default();
    let j = mk_job(&cfg, u.clone(), 0, 0);
    assert!(j.deadline.is_none(), "default config must not expire jobs");
    let mut cfg2 = Config::default();
    cfg2.scheduling.job_max_age_secs = 90;
    let j2 = mk_job(&cfg2, u, 3, 1);
    assert!(
        j2.deadline.is_some(),
        "cap set => every intake path gets a deadline"
    );
    assert_eq!(j2.depth, 3);
    assert_eq!(j2.target, 1);
}

#[test]
fn summary_serializes_pipeline_counters_for_history() {
    let s = Summary {
        pages_fetched: 1,
        requests_total: 1,
        records: 1,
        written: 0,
        deduped: 0,
        blocked: 0,
        retries: 0,
        robots_denied: 0,
        ckpt_writes: 0,
        revalidated: 0,
        quarantined: 1,
        schema_drifts: 2,
        duration_ms: 0,
        per_target: Default::default(),
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["quarantined"], 1);
    assert_eq!(v["schema_drifts"], 2);
    assert_eq!(v["per_target"], serde_json::json!({}));
}
