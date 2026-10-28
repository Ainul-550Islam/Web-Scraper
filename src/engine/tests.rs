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
fn dlq_serialization_redacts_credentials_and_secret_tails() {
    let secret_url = "https://alice:long-password@example.test/private/path?token=secret-token";
    let entry = safe_dlq_entry(
        &Config::default(),
        secret_url,
        serde_json::json!({
            "url": secret_url,
            "reason": "failed request to https://alice:long-password@example.test/private/path?token=secret-token"
        }),
    );
    let text = entry.to_string();
    assert_eq!(entry["url"], "https://example.test");
    assert_eq!(entry["retryable"], false);
    assert!(!text.contains("alice"));
    assert!(!text.contains("long-password"));
    assert!(!text.contains("private/path"));
    assert!(!text.contains("secret-token"));
}

#[test]
fn dlq_retry_resolves_target_identity_and_legacy_queue_prefix_safely() {
    let cfg = Config {
        targets: vec![
            crate::config::TargetCfg {
                id: "catalog".into(),
                start_url: "https://example.test/shop/".into(),
                ..Default::default()
            },
            crate::config::TargetCfg {
                id: "news".into(),
                start_url: "https://news.test/".into(),
                ..Default::default()
            },
        ],
        ..Config::default()
    };
    let approved = Url::parse("https://example.test/shop/item").unwrap();
    assert_eq!(
        target_index_for_dlq(&cfg, &serde_json::json!({"target":"catalog"}), &approved),
        Some(0)
    );
    // Distributed entries from older workers stored the Redis prefix here.
    assert_eq!(
        target_index_for_dlq(&cfg, &serde_json::json!({"target":"crawl:demo"}), &approved),
        Some(0)
    );
    assert_eq!(
        target_index_for_dlq(&cfg, &serde_json::json!({}), &approved),
        Some(0)
    );
    let outside = Url::parse("https://attacker.test/private").unwrap();
    assert_eq!(
        target_index_for_dlq(&cfg, &serde_json::json!({"target":"crawl:demo"}), &outside),
        None
    );
}

#[test]
fn dlq_serialization_keeps_safe_urls_retryable() {
    let safe_url = "https://example.test/path?page=2";
    let entry = safe_dlq_entry(
        &Config::default(),
        safe_url,
        serde_json::json!({"url": safe_url, "reason": "failed: timeout", "attempts": 1}),
    );
    assert_eq!(entry["url"], safe_url);
    assert!(entry.get("retryable").is_none());
}

#[cfg(unix)]
#[test]
fn dlq_reader_refuses_symlinked_files() {
    let dir = std::env::temp_dir().join(format!("harvest-dlq-read-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim");
    let link = dir.join("dlq.jsonl");
    std::fs::write(&victim, b"private data").unwrap();
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    assert!(read_dlq(&link).is_err());
    std::fs::remove_dir_all(dir).unwrap();
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
