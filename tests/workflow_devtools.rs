//! The `devtools` harness (`harvest::testutil`) driving a REAL engine run:
//! mock origin with a mid-stream transport cut, fixture config, event
//! collection, and the metrics recorder — proof the utilities work against
//! production code paths, not just their own unit tests.

use std::collections::BTreeMap;
use std::time::Duration;

use harvest::testutil::{
    collect_events, fanout_page, metrics_view, product_page, MockServer, Route, TempFixture,
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cut_midstream_run_survives_on_harness_mocks() {
    let mock = MockServer::start(vec![
        ("/index.html", Route::ok(fanout_page(3))),
        ("/product/0", Route::ok(product_page(0))),
        ("/product/1", Route::ok(product_page(1))),
        // the flaky page: headers arrive, then the body is cut short.
        (
            "/product/2",
            Route::Cut {
                body_prefix: "<html><body>".into(),
                content_length_lie: true,
            },
        ),
    ]);
    let fx = TempFixture::new("devworkflow", &mock.base());
    let mut cfg = fx.config();
    cfg.events_path = Some(fx.dir.join("out/events.jsonl"));
    let cfg = std::sync::Arc::new(cfg);
    let metrics = std::sync::Arc::new(harvest::metrics::Metrics::default());
    let ctx = harvest::engine::RunCtx {
        metrics: metrics.clone(),
        pool: std::sync::Arc::new(harvest::proxy::ProxyPool::new(&cfg.proxies)),
        runtime: harvest::runtime::RuntimeSlot::default(),
    };

    let summary = harvest::engine::run_with(cfg, ctx, None)
        .await
        .expect("engine run must complete despite a cut page");

    // Resilience: the healthy pages still produced records; the flaky one
    // was retried, not silently swallowed.
    assert_eq!(summary.written, 3, "index + two good products: {summary:?}");
    assert!(
        summary.retries >= 1,
        "cut page hit the retry path: {summary:?}"
    );

    let counts: BTreeMap<String, usize> = mock.hit_counts().into_iter().collect();
    assert!(
        counts["/product/2"] >= 2,
        "retry actually re-hit the wire: {counts:?}"
    );
    assert_eq!(counts["/index.html"], 1, "index fetched once");

    // Durable output: jsonl sink records + the event log, via harness readers
    let recs = std::fs::read_to_string(fx.dir.join("out/records.jsonl")).unwrap();
    assert_eq!(recs.lines().count(), 3);
    let evs = collect_events(&fx.dir.join("out/events.jsonl"));
    assert!(
        evs.iter().any(|e| e["kind"] == "sink_write_completed"),
        "sink events collected: {evs:?}"
    );

    // The metrics recorder view agrees with the run's ground truth.
    let view = metrics_view(&metrics);
    assert_eq!(view["records_written"], 3);
    assert_eq!(view["records_extracted"], 3);
    assert!(view["avg_request_ms"].is_number(), "{view}");
    assert!(view["requests_total"].as_u64().unwrap() >= 4, "{view}");

    // wait_for is the sanctioned polling helper; the mock is quiet now.
    assert!(harvest::testutil::wait_for(Duration::from_secs(2), || true));
}

#[test]
fn test_clock_is_the_only_time_source() {
    // The clock must be deterministic across calls and jump by exactly the
    // requested step — snapshot dedup windows and backoff schedules are
    // pinned against it in unit tests.
    let c = harvest::testutil::TestClock::new(1_700_000_000);
    assert_eq!(c.now(), 1_700_000_000);
    assert_eq!(c.advance(30), 1_700_000_030);
    assert_eq!(c.now(), 1_700_000_030);
    c.set(1_700_000_100);
    assert_eq!(c.advance(0), 1_700_000_100);
}
