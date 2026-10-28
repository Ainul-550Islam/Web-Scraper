//! Resource budgets end-to-end: the record cap drains a run gracefully at
//! the limit, and a proxy in-flight cap converts excess demand into the
//! existing retry/backoff lane — no new failure mode, no data loss.

use std::sync::Arc;

use harvest::testutil::{fanout_page, product_page, MockServer, Route, TempFixture};

fn mock(n: usize) -> MockServer {
    mock_with_delay(n, std::time::Duration::ZERO)
}

/// `delay` makes holds observable: with an in-flight cap, a mocked page
/// that takes real time forces the contention the backpressure test
/// depends on (an instant mock overlaps two workers only for microseconds).
fn mock_with_delay(n: usize, delay: std::time::Duration) -> MockServer {
    let mut all: Vec<(String, Route)> =
        vec![("/index.html".to_string(), Route::ok(fanout_page(n)))];
    for i in 0..n {
        let route = if delay.is_zero() {
            Route::ok(product_page(i))
        } else {
            Route::Delay(delay, Box::new(Route::ok(product_page(i))))
        };
        all.push((format!("/product/{i}"), route));
    }
    let routes: Vec<(&str, Route)> = all.iter().map(|(p, r)| (p.as_str(), r.clone())).collect();
    MockServer::start(routes)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn record_cap_drains_gracefully() {
    let mock = mock(5); // index + 5 products = 6 pages
    let fx = TempFixture::new("budgetcap", &mock.base());
    let mut cfg = fx.config();
    cfg.max_records_per_run = 2;
    cfg.concurrency = 3;
    let cfg = Arc::new(cfg);
    let metrics = Arc::new(harvest::metrics::Metrics::default());
    let ctx = harvest::engine::RunCtx {
        metrics: metrics.clone(),
        pool: Arc::new(harvest::proxy::ProxyPool::new(&cfg.proxies)),
        runtime: harvest::runtime::RuntimeSlot::default(),
    };
    let summary = harvest::engine::run_with(cfg, ctx, None)
        .await
        .expect("a capped run must still succeed");
    assert_eq!(summary.written, 2, "the cap is exact: {summary:?}");
    let j = metrics.display_json();
    assert!(
        j["budget_denied_records"].as_u64().unwrap() >= 1,
        "denials counted: {j}"
    );
    // unlimited control: the same config without the cap writes everything
    let fx2 = TempFixture::new("budgetcap2", &mock.base());
    let mut cfg2 = fx2.config();
    cfg2.concurrency = 3;
    let ctx2 = harvest::engine::RunCtx::default();
    let full = harvest::engine::run_with(Arc::new(cfg2), ctx2, None)
        .await
        .unwrap();
    assert_eq!(full.written, 6, "control run uncapped: {full:?}");
    assert!(
        summary.pages_fetched <= full.pages_fetched,
        "a capped run never does MORE page work than the uncapped one"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn proxy_inflight_cap_becomes_backpressure() {
    let mock = mock_with_delay(4, std::time::Duration::from_millis(60));
    let fx = TempFixture::new("budgetproxy", &mock.base());
    let mut cfg = fx.config();
    cfg.max_inflight_per_proxy = 1;
    cfg.concurrency = 3;
    // The default per-host pace would serialize this single-origin crawl
    // (two requests never overlap => no cap contention to observe). This
    // test is about the proxy budget, so lift the pace gate deliberately.
    cfg.default_policy.requests_per_sec = 1000.0;
    cfg.default_policy.burst = 8;
    cfg.default_policy.min_interval_ms = 0;
    cfg.request_timeout_secs = 8;
    // The cap converts contention into retries: give the retry lane enough
    // attempts and a fast (but nonzero) curve so this test stays quick.
    cfg.max_retries = 24;
    cfg.retry_base_ms = 2;
    let cfg = Arc::new(cfg);
    let metrics = Arc::new(harvest::metrics::Metrics::default());
    let pool = Arc::new(harvest::proxy::ProxyPool::new(&cfg.proxies));
    pool.set_inflight_cap(1);
    let ctx = harvest::engine::RunCtx {
        metrics: metrics.clone(),
        pool: pool.clone(),
        runtime: harvest::runtime::RuntimeSlot::default(),
    };
    let summary = harvest::engine::run_with(cfg, ctx, None)
        .await
        .expect("cap must throttle, not break");
    assert_eq!(summary.written, 5, "all pages still land: {summary:?}");
    assert!(
        pool.denied() >= 1,
        "the cap was actually hit under concurrency 3: {:?}",
        pool.denied()
    );
    let j = metrics.display_json();
    assert!(
        j["budget_denied_proxy"].as_u64().unwrap() >= 1,
        "proxy denials are counted: {j}"
    );
    assert_eq!(pool.inflight(), vec![0], "every hold was released");
    // the snapshot exposes live occupancy for operators
    let snap = pool.snapshot();
    assert_eq!(snap[0].inflight, 0);
    // over-cap denial never wedges the pool: another hold still works
    let h = pool.try_hold(0).expect("idle pool admits again");
    assert_eq!(pool.snapshot()[0].inflight, 1);
    drop(h);
}

#[test]
fn pool_cap_semantics_are_exact() {
    let pool = Arc::new(harvest::proxy::ProxyPool::new(&[]));
    assert_eq!(pool.inflight_cap(), 0, "default: unlimited");
    assert!(pool.try_hold(0).is_some(), "unlimited always admits");
    pool.set_inflight_cap(1);
    let h = pool.try_hold(0).unwrap();
    assert!(pool.try_hold(0).is_none(), "at cap");
    assert_eq!(pool.denied(), 1);
    drop(h);
    assert!(pool.try_hold(0).is_some(), "released capacity re-admits");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_global_page_budget_is_hard_under_fanout() {
    let origin = mock(40);
    let fx = TempFixture::new("auditglobalpages", &origin.base());
    let mut cfg = fx.config();
    cfg.max_pages = 3;
    cfg.targets[0].max_pages = 100;
    cfg.concurrency = 4;
    cfg.default_policy.requests_per_sec = 1000.0;
    cfg.default_policy.burst = 100;
    cfg.default_policy.min_interval_ms = 0;
    let result = harvest::engine::run_with(Arc::new(cfg), harvest::engine::RunCtx::default(), None)
        .await
        .unwrap();
    assert!(
        result.pages_fetched <= 3,
        "queued fanout must not overshoot the global budget: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn audit_target_page_budget_is_hard_under_fanout() {
    let origin = mock(40);
    let fx = TempFixture::new("audittargetpages", &origin.base());
    let mut cfg = fx.config();
    cfg.max_pages = 100;
    cfg.targets[0].max_pages = 3;
    cfg.concurrency = 4;
    cfg.default_policy.requests_per_sec = 1000.0;
    cfg.default_policy.burst = 100;
    cfg.default_policy.min_interval_ms = 0;
    let result = harvest::engine::run_with(Arc::new(cfg), harvest::engine::RunCtx::default(), None)
        .await
        .unwrap();
    assert!(
        result.pages_fetched <= 3,
        "queued fanout must not overshoot the target budget: {result:?}"
    );
}
