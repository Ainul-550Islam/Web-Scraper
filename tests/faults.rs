//! Fault-injection integration: the deterministic mock routes (`flaky`,
//! `Reset`, `hang`) drive the retry/backoff/timeout lanes of a real engine
//! run — transient faults get absorbed, permanent ones degrade the run
//! cleanly instead of panicking or hanging.

use std::sync::Arc;
use std::time::Duration;

use harvest::testutil::{fanout_page, product_page, MockServer, Route, TempFixture};

fn fast_politeness(cfg: &mut harvest::Config) {
    cfg.default_policy.requests_per_sec = 1000.0;
    cfg.default_policy.burst = 8;
    cfg.default_policy.min_interval_ms = 0;
    cfg.retry_base_ms = 5;
    cfg.request_timeout_secs = 6;
}

async fn crawl(mut cfg: harvest::Config) -> (harvest::engine::Summary, serde_json::Value) {
    fast_politeness(&mut cfg);
    let cfg = Arc::new(cfg);
    let ctx = harvest::engine::RunCtx {
        metrics: Arc::new(harvest::metrics::Metrics::default()),
        ..Default::default()
    };
    let metrics = ctx.metrics.clone();
    let s = harvest::engine::run_with(cfg, ctx, None)
        .await
        .expect("fault-tolerant runs must finish, not fail");
    (s, metrics.display_json())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transient_flakes_are_absorbed_by_retries() {
    // every product 503s twice, then succeeds: the retry lane must recover
    // every record, with retries visible in metrics and per-path hit counts.
    let n = 4;
    let mut all: Vec<(String, Route)> =
        vec![("/index.html".to_string(), Route::ok(fanout_page(n)))];
    for i in 0..n {
        all.push((
            format!("/product/{i}"),
            Route::flaky(2, 503, Route::ok(product_page(i))),
        ));
    }
    let routes: Vec<(&str, Route)> = all.iter().map(|(p, r)| (p.as_str(), r.clone())).collect();
    let mock = MockServer::start(routes);
    let fx = TempFixture::new("flaky-ok", &mock.base());
    let mut cfg = fx.config();
    cfg.concurrency = 3;
    cfg.max_retries = 4;
    let (s, m) = crawl(cfg).await;
    assert_eq!(
        s.written,
        n as u64 + 1,
        "flakes must cost nothing but time: {s:?}"
    );
    assert!(m["requests_retry"].as_u64().unwrap() >= (2 * n) as u64);
    let counts = mock.hit_counts();
    for i in 0..n {
        assert_eq!(
            counts[&format!("/product/{i}")],
            3,
            "exactly 2 failures + 1 success per product"
        );
    }
}

#[tokio::test]
async fn permanent_failures_degrade_not_crash() {
    // always-503 origin with zero slack: run completes, nothing written,
    // failures counted — an error exit would be the bug.
    let n = 3;
    let mut all: Vec<(String, Route)> =
        vec![("/index.html".to_string(), Route::ok(fanout_page(n)))];
    for i in 0..n {
        all.push((format!("/product/{i}"), Route::status(503, "gone")));
    }
    let routes: Vec<(&str, Route)> = all.iter().map(|(p, r)| (p.as_str(), r.clone())).collect();
    let mock = MockServer::start(routes);
    let fx = TempFixture::new("always-503", &mock.base());
    let mut cfg = fx.config();
    cfg.max_retries = 1;
    let (s, m) = crawl(cfg).await;
    assert_eq!(s.written, 1, "only the index record: {s:?}");
    assert!(m["jobs_failed"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn closed_connections_and_hangs_hit_transport_lanes() {
    // /p0 closes without answering; /p1 hangs past the request timeout.
    // Both must surface as (retryable) transport errors, and the run must
    // terminate on the timeout clock, not the socket's.
    let all: Vec<(String, Route)> = vec![
        ("/index.html".to_string(), Route::ok(fanout_page(2))),
        ("/product/0".to_string(), Route::Reset),
        // Longer than the client timeout, bounded so the mock's accept
        // loop recovers quickly after the client gives up: the run must
        // terminate on the timeout clock, not the socket's.
        (
            "/product/1".to_string(),
            Route::hang(Duration::from_secs(2)),
        ),
    ];
    let routes: Vec<(&str, Route)> = all.iter().map(|(p, r)| (p.as_str(), r.clone())).collect();
    let mock = MockServer::start(routes);
    let fx = TempFixture::new("transport-faults", &mock.base());
    let mut cfg = fx.config();
    cfg.request_timeout_secs = 1;
    cfg.max_retries = 1;
    let started = std::time::Instant::now();
    let (s, m) = crawl(cfg).await;
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "timeout must bound the hang"
    );
    assert_eq!(s.written, 1, "products stay unfetched: {s:?}");
    let retried = m["requests_retry"].as_u64().unwrap();
    assert!(
        retried >= 2,
        "both transport faults retried at least once: {retried}"
    );
}

#[tokio::test]
async fn transport_breaker_abandons_host_on_repeated_resets() {
    // Every request resets; the transport breaker must give up on the
    // domain instead of grinding the retry budget through every page.
    let n = 6;
    let mut all: Vec<(String, Route)> =
        vec![("/index.html".to_string(), Route::ok(fanout_page(n)))];
    for i in 0..n {
        all.push((format!("/product/{i}"), Route::Reset));
    }
    let routes: Vec<(&str, Route)> = all.iter().map(|(p, r)| (p.as_str(), r.clone())).collect();
    let mock = MockServer::start(routes);
    let fx = TempFixture::new("breaker-abandon", &mock.base());
    let mut cfg = fx.config();
    cfg.max_retries = 0; // one attempt each: every failure lands on the breaker
    cfg.concurrency = 1; // serial, so the failures are truly consecutive
    cfg.transport_breaker = Some(harvest::config::BreakerOpts {
        min_consecutive: 3,
        cooldown_secs: 600, // stay open for the rest of this run
        recovery_successes: 1,
    });
    let (s, m) = crawl(cfg).await;
    let rejected = m["breaker_rejected"].as_u64().unwrap();
    assert!(
        rejected >= 1,
        "breaker must open and short-circuit: {s:?} {m}"
    );
    // index + exactly the 3 striking pages got sockets; the rest were skipped
    let counts = mock.hit_counts();
    let opened: usize = (0..n)
        .map(|i| counts.get(&format!("/product/{i}")).copied().unwrap_or(0))
        .sum();
    assert!(
        opened <= 3,
        "later jobs must not touch the network: {counts:?}"
    );
}
