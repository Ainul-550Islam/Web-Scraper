use super::*;

#[test]
fn clock_is_pure_and_deterministic() {
    let c = TestClock::new(1000);
    assert_eq!(c.now(), 1000);
    assert_eq!(c.advance(30), 1030);
    c.set(7);
    assert_eq!(c.now(), 7);
}

#[test]
fn mock_server_routes_hits_and_modes() {
    let m = MockServer::start(vec![
        ("/index.html", Route::ok(fanout_page(2))),
        ("/product/", Route::ok(product_page(7))),
        (
            "/slow",
            Route::Delay(Duration::from_millis(150), Box::new(Route::ok("late"))),
        ),
        (
            "/cut",
            Route::Cut {
                body_prefix: "<html>partial".into(),
                content_length_lie: true,
            },
        ),
        ("/bad", Route::Garbage),
        (
            "/redir",
            Route::Redirect(301, "http://example.com/x".into()),
        ),
    ]);
    let fetch = |path: &str| {
        let mut s = TcpStream::connect(&m.addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nhost: x\r\n\r\n").as_bytes())
            .unwrap();
        let mut buf = Vec::new();
        let _ = s.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).into_owned()
    };
    let idx = fetch("/index.html");
    assert!(idx.contains("200") && idx.contains("p0</a>") && idx.contains("p1</a>"));
    assert!(fetch("/product/3").contains("P7"));
    assert!(fetch("/nope").contains("404"));
    let t0 = Instant::now();
    assert!(fetch("/slow").contains("late"));
    assert!(
        t0.elapsed() >= Duration::from_millis(140),
        "delay must delay"
    );
    let cut = fetch("/cut");
    assert!(cut.contains("content-length: 1013") && cut.contains("partial"));
    assert!(fetch("/bad").contains("NOT-HTTP") || fetch("/bad").is_empty());
    assert!(fetch("/redir").contains("location: http://example.com/x"));
    let counts = m.hit_counts();
    assert!(counts.get("/index.html") >= Some(&1));
    assert!(counts.contains_key("/nope"), "404s are recorded too");
}

#[test]
fn in_memory_sink_records_and_flushes() {
    let s = InMemorySink::new();
    let mut fields = Map::new();
    fields.insert("title".into(), json!("T"));
    let rec = Record {
        run: "r".into(),
        target: "t".into(),
        url: "http://x/1".into(),
        depth: 1,
        ts: 42,
        fingerprint: "fp".into(),
        egress: "direct".into(),
        fields,
    };
    Sink::write(&s, &rec).unwrap();
    Sink::flush(&s).unwrap();
    let snap = s.snapshot();
    assert_eq!(snap.len(), 1);
    assert_eq!(snap[0]["title"], "T");
    assert_eq!(s.flushes.load(Ordering::Relaxed), 1);
}

#[test]
fn fixture_round_trips_through_real_loading() {
    let m = MockServer::start(vec![("/index.html", Route::ok(fanout_page(1)))]);
    let fx = TempFixture::new("roundtrip", &m.base());
    let cfg = fx.config();
    assert_eq!(cfg.targets.len(), 1);
    assert!(!cfg.respect_robots);
    assert!(fx.path().join("out").exists());
}

#[test]
fn wait_for_terminates_both_ways() {
    let ok = wait_for(Duration::from_millis(50), || true);
    assert!(ok);
    let timeout = wait_for(Duration::from_millis(30), || false);
    assert!(!timeout, "must not hang");
}

#[test]
fn metrics_view_adds_derived_means() {
    let m = crate::metrics::Metrics::default();
    m.note_request_ms(10);
    m.note_request_ms(30);
    m.request_started();
    m.request_started();
    m.note_storage_write(8);
    let v = metrics_view(&m);
    assert_eq!(v["avg_request_ms"], 20.0);
    assert_eq!(v["avg_storage_write_ms"], 8.0);
}
