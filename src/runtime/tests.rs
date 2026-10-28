use super::*;

fn cfg() -> Config {
    Config {
        targets: vec![crate::config::TargetCfg {
            id: "t".into(),
            start_url: "http://x/".into(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn snapshot_shape_is_stable_and_self_describing() {
    let rt = Runtime::new(&cfg(), Arc::new(Metrics::default()));
    let v = rt.snapshot(false);
    assert_eq!(v["engine"]["run"], "harvest");
    assert_eq!(v["engine"]["status"], "idle");
    assert_eq!(v["queue"], Value::Null, "no live queue attached");
    assert_eq!(v["events"]["published"], 0);
    assert!(v["accounting"]["bytes_received"].is_number());
    rt.emit(EventKind::JobStarted, Some(1), json!({"url":"http://x/1"}));
    let v2 = rt.snapshot(true);
    assert_eq!(v2["engine"]["status"], "running");
    assert_eq!(v2["events"]["published"], 1);
    assert_eq!(v2["events"]["last_seq"], 1);
}

#[tokio::test]
async fn events_log_persists_redacted_urls_to_a_private_file() {
    let dir = std::env::temp_dir().join(format!("harvest-events-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("nested/events.jsonl");
    let bus = crate::events::Bus::new(16);
    bus.publish(
        EventKind::JobStarted,
        Some(1),
        json!({"url":"https://alice:pw@example.test/private?token=secret"}),
    );
    bus.close();

    assert_eq!(events_log_writer(bus, path.clone()).await.unwrap(), 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.contains("alice"));
    assert!(!text.contains("pw"));
    assert!(!text.contains("private"));
    assert!(!text.contains("secret"));
    assert!(text.contains("https://example.test"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn breakers_lazy_and_shared_per_host() {
    let mut c = cfg();
    c.transport_breaker = Some(crate::config::BreakerOpts {
        min_consecutive: 2,
        cooldown_secs: 5,
        recovery_successes: 1,
    });
    let rt = Runtime::new(&c, Arc::new(Metrics::default()));
    let a = rt.breaker_for("h1").expect("configured -> Some");
    let b = rt.breaker_for("h1").expect("same host, same breaker");
    assert!(Arc::ptr_eq(&a, &b));
    assert!(rt.breaker_for("h2").is_some());
    // Below threshold the host is still Closed -> omitted from the
    // snapshot (snapshots stay small: only interesting hosts).
    a.note_failure(Instant::now());
    assert!(rt.snapshot(false)["breakers"]
        .as_object()
        .unwrap()
        .is_empty());
    a.note_failure(Instant::now()); // threshold is 2 -> open
    let snap = rt.snapshot(false);
    let br = snap["breakers"].as_object().unwrap();
    assert!(br.contains_key("h1"));
    assert_eq!(br["h1"]["state"], "open");
}

#[test]
fn breaker_absent_when_feature_unconfigured() {
    let rt = Runtime::new(&cfg(), Arc::new(Metrics::default()));
    assert!(rt.breaker_for("h").is_none(), "default-off means zero cost");
}

#[test]
fn shutdown_events_emitted_once() {
    let rt = Runtime::new(&cfg(), Arc::new(Metrics::default()));
    rt.begin_shutdown();
    rt.begin_shutdown();
    assert_eq!(rt.snapshot(true)["events"]["published"], 1, "idempotent");
    // Drain marker WITHOUT a stop request: still "running" — the crawl
    // is actively fetching; "draining" is reserved for a real stop.
    assert_eq!(rt.snapshot(true)["engine"]["status"], "running");
    let sup = crate::supervisor::Supervisor::new(
        crate::supervisor::RestartPolicy::default(),
        crate::supervisor::CancelToken::new(),
    );
    rt.attach_supervisor(sup.clone());
    sup.begin_drain();
    assert_eq!(
        rt.snapshot(true)["engine"]["status"],
        "draining",
        "after stop"
    );
    rt.end_shutdown(&json!({"written": 3}));
    let t = rt.bus.tail(4);
    assert_eq!(t.last().map(|e| e.kind), Some(EventKind::ShutdownCompleted));
}
