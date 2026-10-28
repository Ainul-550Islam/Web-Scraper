//! Engine-layer integration: job lifecycle events, scheduler policies,
//! transport breaker, deadlines, retry determinism, runtime snapshot over
//! the admin server, and drain semantics — all against in-process canned
//! HTTP servers so the suite is deterministic and needs no external
//! services.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use harvest::admin::AdminState;
use harvest::engine::{self, RunCtx};
use harvest::metrics::Metrics;
use harvest::proxy::ProxyPool;
use harvest::runtime::{Runtime, RuntimeSlot};
use harvest::Config;

/// Canned server modes:
/// * every path under `/product/` can be made to blackhole (accept then
///   close, no bytes) to produce *transport* failures for breaker tests;
/// * a fixed per-request delay produces slow-response conditions;
/// * index fans out to N unique product pages for queue-shape tests.
struct MockOpts {
    fanout: usize,
    product_blackhole: bool,
    delay: Duration,
}

fn spawn_mock(opts: MockOpts) -> (String, Arc<Mutex<HashMap<String, usize>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits: Arc<Mutex<HashMap<String, usize>>> = Default::default();
    let h = hits.clone();
    let MockOpts {
        fanout,
        product_blackhole,
        delay,
    } = opts;
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let mut reader = BufReader::new(match stream.try_clone() {
                Ok(s) => s,
                Err(_) => continue,
            });
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            *h.lock().unwrap().entry(path.clone()).or_default() += 1;
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err()
                    || hdr == "\n"
                    || hdr == "\r\n"
                    || hdr.is_empty()
                {
                    break;
                }
            }
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
            if product_blackhole && path.starts_with("/product/") {
                // The "server" dies mid-handshake: reqwest sees a transport
                // error, which is exactly the class the circuit breaker
                // governs.
                let _ = stream.shutdown(std::net::Shutdown::Both);
                continue;
            }
            let (status, body) = if path == "/index.html" {
                let links: Vec<String> = (0..fanout)
                    .map(|i| format!("<a class=product href=/product/{i}>p{i}</a>"))
                    .collect();
                (
                    "200 OK",
                    format!(
                        "<html><body><h1>Index</h1><p class=price>$1.00</p>{}</body></html>",
                        links.concat()
                    ),
                )
            } else if let Some(n) = path.strip_prefix("/product/") {
                (
                    "200 OK",
                    format!("<html><body><h1>P{n}</h1><p class=price>${n}.50</p></body></html>"),
                )
            } else if path.starts_with("/product") {
                ("200 OK", "<html><body><h1>P</h1></body></html>".to_string())
            } else {
                ("404 Not Found", "<html><body>404</body></html>".to_string())
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            let _ = stream.flush();
        }
    });
    (format!("http://127.0.0.1:{port}"), hits)
}

fn cfg_with(base: &str, out: &std::path::Path, extra: serde_json::Value) -> Config {
    let mut root = serde_json::json!({
        "name": "engine-layer",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": false,
        "concurrency": 1,
        "max_retries": 0,
        "retry_base_ms": 5,
        "max_pages": 50,
        "default_policy": { "requests_per_sec": 5000.0, "burst": 100, "min_interval_ms": 0 },
        "metrics_addr": "",
        "targets": [{
            "id": "demo",
            "start_url": format!("{base}/index.html"),
            "max_depth": 2,
            "follow_links": true,
            "max_pages": 25,
            "extract": { "title": { "css": "h1" }, "price": { "css": "p.price" } }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    if let (Some(obj), Some(add)) = (root.as_object_mut(), extra.as_object()) {
        for (k, v) in add {
            obj.insert(k.clone(), v.clone());
        }
    }
    let mut cfg: Config = serde_json::from_value(root).unwrap();
    cfg.validate().unwrap();
    cfg
}

fn tmpdir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("harvest-engine-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn read_events(path: &std::path::Path) -> Vec<serde_json::Value> {
    let txt = std::fs::read_to_string(path).unwrap_or_default();
    txt.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("event line is JSON"))
        .collect()
}

#[tokio::test]
async fn lifecycle_events_stream_end_to_end() {
    let (url, _hits) = spawn_mock(MockOpts {
        fanout: 2,
        product_blackhole: false,
        delay: Duration::ZERO,
    });
    let dir = tmpdir("lifecycle");
    let events = dir.join("events.jsonl");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({"events_path": events.to_str().unwrap()}),
    );
    let summary = engine::run(cfg).await.expect("run ok");
    assert!(summary.written >= 3, "index + 2 products: {summary:?}");

    let evs = read_events(&events);
    let kinds: Vec<&str> = evs
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or(""))
        .collect();
    for needle in [
        "job_submitted",
        "job_started",
        "request_started",
        "proxy_selected",
        "request_finished",
        "extraction_completed",
        "sink_write_completed",
        "job_completed",
        "shutdown_started",
        "shutdown_completed",
    ] {
        assert!(kinds.contains(&needle), "missing {needle} in {kinds:?}");
    }
    // seq strictly increasing and every line carries a timestamp
    let seqs: Vec<u64> = evs.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert!(
        seqs.windows(2).all(|w| w[1] == w[0] + 1),
        "gapped seq: {seqs:?}"
    );
    assert!(evs.iter().all(|e| e["ts"].as_u64().unwrap_or(0) > 0));
    // The final event is the shutdown marker: the run owns its log.
    assert_eq!(kinds.last().copied(), Some("shutdown_completed"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn transport_breaker_opens_and_skips_further_fetches() {
    let (url, hits) = spawn_mock(MockOpts {
        fanout: 3,
        product_blackhole: true,
        delay: Duration::ZERO,
    });
    let dir = tmpdir("breaker");
    let events = dir.join("events.jsonl");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "events_path": events.to_str().unwrap(),
            "transport_breaker": { "min_consecutive": 2, "cooldown_secs": 3600, "recovery_successes": 1 },
        }),
    );
    let metrics = Arc::new(Metrics::default());
    let slot = RuntimeSlot::default();
    let summary = engine::run_with(
        Arc::new(cfg),
        RunCtx {
            metrics: metrics.clone(),
            pool: Arc::new(ProxyPool::new(&[])),
            runtime: slot.clone(),
        },
        None,
    )
    .await
    .expect("run completes despite the breaker");
    assert!(summary.written >= 1, "index itself was written");

    // Exactly two products were attempted (each tripping the breaker);
    // the third was skipped without touching the socket.
    let attempted = hits
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.starts_with("/product/"))
        .map(|(_, n)| *n)
        .sum::<usize>();
    assert_eq!(attempted, 2, "breaker must save the third request");
    assert_eq!(metrics.breaker_rejected.load(Ordering::Relaxed), 1);

    let evs = read_events(&events);
    let kinds: Vec<&str> = evs
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(kinds.contains(&"breaker_opened"), "{kinds:?}");

    // The runtime snapshot still shows the open circuit after the run.
    let rt = slot.get().expect("runtime attached");
    let snap = rt.snapshot(false);
    // The breaker keys on the URL host (no port) — same identity the
    // politeness layer uses, deliberately.
    let host = url
        .trim_start_matches("http://")
        .split(':')
        .next()
        .unwrap()
        .to_string();
    assert!(
        snap["breakers"].get(&host).is_some(),
        "open host must appear: {snap:#}"
    );
    assert_eq!(snap["breakers"][&host]["state"], "open");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn bounded_queue_applies_backpressure_without_stalling() {
    let (url, _hits) = spawn_mock(MockOpts {
        fanout: 6,
        product_blackhole: false,
        delay: Duration::ZERO,
    });
    let dir = tmpdir("bounded");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({"scheduling": { "max_queue_depth": 2 }}),
    );
    let metrics = Arc::new(Metrics::default());
    let summary = engine::run_with(
        Arc::new(cfg),
        RunCtx {
            metrics: metrics.clone(),
            pool: Arc::new(ProxyPool::new(&[])),
            runtime: RuntimeSlot::default(),
        },
        None,
    )
    .await
    .expect("bounded queue must never wedge the run");
    let rejected = metrics.queue_rejected.load(Ordering::Relaxed);
    assert!(rejected >= 3, "6 links vs depth 2: {rejected} rejected");
    // Whatever fit in the queue still gets processed — backpressure drops
    // intake, never corrupts work in flight.
    assert!(summary.written >= 2, "{summary:?}");
    // And the run reached a clean completion event via accounting.
    assert!(metrics.jobs_completed.load(Ordering::Relaxed) >= 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn queued_jobs_expire_under_job_max_age() {
    let (url, hits) = spawn_mock(MockOpts {
        fanout: 3,
        product_blackhole: false,
        delay: Duration::from_millis(1100),
    });
    let dir = tmpdir("deadline");
    let events = dir.join("events.jsonl");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "events_path": events.to_str().unwrap(),
            "scheduling": { "job_max_age_secs": 1 },
        }),
    );
    let summary = engine::run(cfg).await.expect("run ok");
    // Index (queued at t0) ran first; the three children queued behind a
    // 1.1s response are older than their 1s budget when popped — they
    // expire instead of crawling stale work.
    let evs = read_events(&events);
    let expired = evs.iter().filter(|e| e["kind"] == "job_expired").count();
    assert!(expired >= 2, "expected >=2 expirations, got {expired}");
    assert!(
        summary.written <= 3,
        "expired jobs never write: {summary:?}"
    );
    // Expired children never fetched — the server saw only the index.
    let product_fetches: usize = hits
        .lock()
        .unwrap()
        .iter()
        .filter(|(p, _)| p.starts_with("/product/"))
        .map(|(_, n)| *n)
        .sum();
    assert!(
        product_fetches <= 1,
        "one at most could start before expiry: {product_fetches}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn deterministic_retry_mode_yields_exact_delay_curve() {
    // A dedicated always-500 server: the retry loop is what this test
    // exercises, so the simplest fixture is a host that never recovers.
    let hits: Arc<Mutex<HashMap<String, usize>>> = Default::default();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port500 = listener.local_addr().unwrap().port();
    let h5 = hits.clone();
    std::thread::spawn(move || {
        let hits = h5;
        for conn in listener.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut buf = [0u8; 1024];
            let Ok(n) = s.read(&mut buf) else { continue };
            let head = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            *hits.lock().unwrap().entry(path.clone()).or_default() += 1;
            if path == "/index.html" {
                let body = "<html><body><h1>x</h1></body></html>";
                let _ = s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes());
            } else {
                let _ = s.write_all(
                    b"HTTP/1.1 500 Oops\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                );
            }
            let _ = s.flush();
        }
    });
    let base = format!("http://127.0.0.1:{port500}");
    let dir = tmpdir("retrydet");
    let events = dir.join("events.jsonl");
    let mut cfg = cfg_with(
        &base,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "events_path": events.to_str().unwrap(),
            "retry_deterministic": true,
            "max_retries": 3,
            "retry_base_ms": 100,
            "max_single_wait_ms": 5000,
            // fan out to one product so a 500 path is exercised
            "targets": [{
                "id": "demo",
                "start_url": format!("{base}/index.html"),
                "max_depth": 1,
                "follow_links": false,
                "max_pages": 5,
                "extract": { "title": { "css": "h1" } }
            }],
        }),
    );
    // Point the start_url directly at the 500 path to force the retry loop.
    cfg.targets[0].start_url = format!("{base}/missing.html");
    let summary = engine::run(cfg).await.expect("run ends despite 500");
    assert_eq!(summary.written, 0);
    assert_eq!(
        hits.lock()
            .unwrap()
            .get("/missing.html")
            .copied()
            .unwrap_or(0),
        4,
        "initial try + 3 retries, exactly"
    );

    let evs = read_events(&events);
    let delays: Vec<u64> = evs
        .iter()
        .filter(|e| e["kind"] == "retry_scheduled")
        .filter_map(|e| e["detail"]["delay_ms"].as_u64())
        .collect();
    // Exact doubling curve with zero jitter: 100, 200, 400 (then exhausted).
    assert_eq!(
        delays,
        vec![100, 200, 400],
        "deterministic curve must be exact"
    );
    let reasons: Vec<&str> = evs
        .iter()
        .filter(|e| e["kind"] == "retry_scheduled")
        .map(|e| e["detail"]["reason"].as_str().unwrap_or("?"))
        .collect();
    assert!(reasons.iter().all(|r| *r == "server-5xx"), "{reasons:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn scheduling_modes_complete_identical_work() {
    let (url, _hits) = spawn_mock(MockOpts {
        fanout: 4,
        product_blackhole: false,
        delay: Duration::ZERO,
    });
    let mut results = Vec::new();
    for mode in ["fifo", "priority", "fair"] {
        let dir = tmpdir(&format!("mode-{mode}"));
        let cfg = cfg_with(
            &url,
            &dir.join("out.jsonl"),
            serde_json::json!({ "scheduling": { "mode": mode } }),
        );
        let summary = engine::run(cfg)
            .await
            .unwrap_or_else(|e| panic!("{mode}: {e}"));
        let lines: Vec<String> = std::fs::read_to_string(dir.join("out.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["url"].as_str().unwrap_or("").to_string()
            })
            .collect();
        let mut lines = lines;
        lines.sort();
        results.push((mode.to_string(), summary.written, lines));
        let _ = std::fs::remove_dir_all(&dir);
    }
    let (_m0, w0, l0) = &results[0];
    for (m, w, l) in &results[1..] {
        assert_eq!(w, w0, "{m} wrote a different count");
        assert_eq!(l, l0, "{m} collected a different URL set");
    }
    assert!(*w0 >= 5, "index + 4 products: {results:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn runtime_snapshot_and_events_over_admin_token_auth() {
    // Free-port grab, then a real serve() on it.
    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let addr = format!("127.0.0.1:{port}");

    let cfg = serde_json::from_value::<Config>(serde_json::json!({
        "targets": [{ "id": "t", "start_url": "http://x/", "extract": { "a": { "css": "h1" } } }],
        "metrics_addr": "",
    }))
    .unwrap();
    let metrics = Arc::new(Metrics::default());
    let rt = Runtime::new(&cfg, metrics.clone());
    let slot = RuntimeSlot::default();
    slot.set(rt.clone());
    let state = Arc::new(AdminState::new(
        metrics.clone(),
        Arc::new(ProxyPool::new(&[])),
        std::path::PathBuf::from("/nonexistent/history.jsonl"),
        Some("harvest-test-token-0123456789".to_string()),
    ));
    state.attach_runtime(slot.clone());
    let server = tokio::spawn(harvest::admin::serve(addr.clone(), state.clone()));
    // let the listener come up
    for _ in 0..50 {
        if std::net::TcpStream::connect(&addr).is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    rt.emit(
        harvest::events::EventKind::JobStarted,
        Some(1),
        serde_json::json!({"url": "http://x/1"}),
    );
    rt.emit(
        harvest::events::EventKind::JobCompleted,
        Some(1),
        serde_json::json!({"url": "http://x/1"}),
    );

    let get = |path: &str, token: Option<&str>| {
        let addr = addr.clone();
        let path = path.to_string();
        let token = token.map(str::to_string);
        std::thread::spawn(move || {
            let mut sock = TcpStream::connect(&addr).expect("connect admin");
            sock.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let auth = token
                .map(|t| format!("authorization: Bearer {t}\r\n"))
                .unwrap_or_default();
            sock.write_all(
                format!("GET {path} HTTP/1.1\r\nhost: {addr}\r\n{auth}connection: close\r\n\r\n")
                    .as_bytes(),
            )
            .unwrap();
            let mut got = String::new();
            sock.read_to_string(&mut got).unwrap();
            got
        })
        .join()
        .expect("probe thread")
    };

    let unauth = get("/runtime", None);
    assert!(unauth.starts_with("HTTP/1.1 401"), "fail-closed: {unauth}");
    let unauth2 = get("/events", None);
    assert!(unauth2.starts_with("HTTP/1.1 401"), "events gated too");

    let tok = "harvest-test-token-0123456789";
    let body = get("/runtime", Some(tok));
    assert!(
        body.starts_with("HTTP/1.1 200"),
        "{}",
        &body[..body.len().min(160)]
    );
    let json: serde_json::Value =
        serde_json::from_str(body.split("\r\n\r\n").nth(1).unwrap()).expect("/runtime json");
    assert_eq!(json["engine"]["run"], "harvest");
    // 2 job events + the admin server's own `serve_start` operator action
    // (mirrored onto the bus because the runtime slot was already attached).
    assert_eq!(json["events"]["published"], 3);
    assert!(json.get("accounting").is_some());
    assert!(
        json.get("queue").is_some(),
        "idle queue is null, key present"
    );
    assert!(
        json["workers"].is_null(),
        "no supervisor attached on this aux runtime"
    );

    let ev = get("/events?limit=1", Some(tok));
    let ej: serde_json::Value =
        serde_json::from_str(ev.split("\r\n\r\n").nth(1).unwrap()).expect("/events json");
    assert_eq!(ej["limit"], 1);
    assert_eq!(ej["events"].as_array().unwrap().len(), 1);
    assert_eq!(ej["events"][0]["kind"], "job_completed", "tail = newest");
    // and the operator action is visible one further back
    let ev3 = get("/events?limit=5", Some(tok));
    let ej3: serde_json::Value =
        serde_json::from_str(ev3.split("\r\n\r\n").nth(1).unwrap()).expect("/events wide json");
    assert_eq!(ej3["events"].as_array().unwrap().len(), 3);
    assert_eq!(
        ej3["events"][2]["kind"], "operator_action",
        "serve_start mirrored to the bus"
    );

    let nf = get("/definitely-not-a-route", Some(tok));
    assert!(nf.starts_with("HTTP/1.1 404"));
    assert!(nf.contains("/runtime"), "help line lists new routes");

    server.abort();
}

#[tokio::test]
async fn drain_flag_produces_clean_shutdown_and_drain_log() {
    let (url, _hits) = spawn_mock(MockOpts {
        fanout: 8,
        product_blackhole: false,
        delay: Duration::from_millis(120),
    });
    let dir = tmpdir("drain");
    let events = dir.join("events.jsonl");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "events_path": events.to_str().unwrap(),
            "max_pages": 40,
        }),
    );
    let stop = Arc::new(AtomicBool::new(false));
    let s2 = stop.clone();
    let stopper = tokio::spawn(async move {
        // Stop shortly after the crawl starts; the point is the *shape* of
        // shutdown (both lifecycle events + no panic), not exact counts.
        tokio::time::sleep(Duration::from_millis(700)).await;
        s2.store(true, Ordering::Relaxed);
    });
    let summary = engine::run_with_stop(cfg, Some(stop), false, false)
        .await
        .expect("drained run is not an error run");
    stopper.await.ok();
    let evs = read_events(&events);
    let kinds: Vec<&str> = evs
        .iter()
        .map(|e| e["kind"].as_str().unwrap_or(""))
        .collect();
    assert!(kinds.contains(&"shutdown_started"));
    assert_eq!(kinds.last().copied(), Some("shutdown_completed"));
    // Drain kept it finite: far fewer than the 40-page budget. Each started
    // job either reached a terminal event or was explicitly returned to the
    // pending queue when operator cancellation interrupted its request.
    assert!(summary.written < 40);
    let started = kinds.iter().filter(|k| **k == "job_started").count();
    let finished = kinds
        .iter()
        .filter(|k| **k == "job_completed" || **k == "job_failed" || **k == "job_expired")
        .count();
    let requeued = evs
        .iter()
        .filter(|event| event["kind"] == "job_submitted" && event["detail"]["requeued"] == true)
        .count();
    assert_eq!(
        started,
        finished + requeued,
        "every started job must finish or be represented as pending on shutdown"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------- pipeline
//
// The transform/schema layer runs through the *real* engine: same job
// plumbing, sink, events — only the record shape between extract and write
// differs. Both scenarios pin the summary counters to exact numbers.

#[tokio::test]
async fn coerce_pipeline_repairs_and_counts_drift_exactly() {
    let (url, _hits) = spawn_mock(MockOpts {
        fanout: 2,
        product_blackhole: false,
        delay: Duration::ZERO,
    });
    let dir = tmpdir("pipeline-coerce");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "targets": [{
                "id": "demo",
                "start_url": format!("{url}/index.html"),
                "max_depth": 2,
                "follow_links": true,
                "max_pages": 25,
                "extract": {
                    "title": {
                        "css": "h1",
                        "transforms": [{ "op": "regex_extract", "pattern": "P?(\\d+)", "group": 1 }]
                    },
                    "price": {
                        "css": "p.price",
                        "transforms": [{ "op": "parse_number" }]
                    }
                },
                "schema": {
                    "enforce": "coerce",
                    "fields": { "title": "int", "price": "number" }
                }
            }]
        }),
    );
    let summary = engine::run(cfg).await.expect("run ok");
    assert_eq!(summary.written, 3, "all pages still write in coerce mode");
    assert_eq!(summary.quarantined, 0);
    // index: regex fails (no digits) + int-check fails  => 2
    // products: "P0"/"P1" extract digits, still text -> int coercion => 1 each
    // price: "$n.50" parses to a number, declared number matches => 0
    assert_eq!(
        summary.schema_drifts, 4,
        "drift must be exactly countable: {summary:?}"
    );

    let out = std::fs::read_to_string(dir.join("out.jsonl")).unwrap();
    let recs: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(recs.len(), 3);
    let product = recs
        .iter()
        .find(|r| r["url"].as_str().unwrap().ends_with("/product/1"))
        .expect("product/1 row");
    assert_eq!(
        product["fields"]["title"],
        serde_json::json!(1),
        "coerced int"
    );
    assert_eq!(
        product["fields"]["price"],
        serde_json::json!(1.5),
        "parsed number"
    );
    let index = recs
        .iter()
        .find(|r| r["url"].as_str().unwrap().ends_with("/index.html"))
        .expect("index row");
    assert_eq!(
        index["fields"]["title"], "Index",
        "failed repair keeps the raw value"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn strict_pipeline_quarantines_and_still_walks_the_graph() {
    let (url, hits) = spawn_mock(MockOpts {
        fanout: 2,
        product_blackhole: false,
        delay: Duration::ZERO,
    });
    let dir = tmpdir("pipeline-strict");
    let events = dir.join("events.jsonl");
    let quarantine = dir.join("quarantine.jsonl");
    let cfg = cfg_with(
        &url,
        &dir.join("out.jsonl"),
        serde_json::json!({
            "events_path": events.to_str().unwrap(),
            "targets": [{
                "id": "demo",
                "start_url": format!("{url}/index.html"),
                "max_depth": 2,
                "follow_links": true,
                "max_pages": 25,
                "extract": {
                    "sku": {
                        "css": "h1",
                        "transforms": [{ "op": "regex_extract", "pattern": "^SKU-(\\d{6})$" }]
                    }
                },
                "schema": {
                    "enforce": "strict",
                    "quarantine_path": quarantine.to_str().unwrap()
                }
            }]
        }),
    );
    let summary = engine::run(cfg)
        .await
        .expect("quarantining must not fail the job");
    assert_eq!(summary.records, 3, "extraction still happened");
    assert_eq!(summary.written, 0, "nothing reaches the sink");
    assert_eq!(summary.quarantined, 3);
    assert_eq!(
        summary.schema_drifts, 0,
        "strict failures are quarantined, not drift"
    );
    // Discovery continues through quarantined pages: all 3 pages fetched.
    let h = hits.lock().unwrap();
    assert_eq!(*h.get("/index.html").unwrap_or(&0), 1);
    assert_eq!(*h.get("/product/0").unwrap_or(&0), 1);
    assert_eq!(*h.get("/product/1").unwrap_or(&0), 1);
    drop(h);

    let q: Vec<serde_json::Value> = std::fs::read_to_string(&quarantine)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(q.len(), 3);
    let joined: Vec<String> = q[0]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        joined
            .iter()
            .any(|r| r.contains("sku") && r.contains("regex_extract") && r.contains("no match")),
        "reasons must name field+step: {joined:?}"
    );
    assert_eq!(q[0]["depth"], 0, "index page is depth 0");
    assert!(
        q[0]["fields"]["sku"].is_string(),
        "raw field survives in quarantine"
    );

    let evs = read_events(&events);
    let n_q = evs
        .iter()
        .filter(|e| e["kind"] == "record_quarantined")
        .count();
    assert_eq!(n_q, 3, "one event per quarantined record");
    let no_writes = evs.iter().all(|e| e["kind"] != "sink_write_completed");
    assert!(no_writes, "sink never ran");
    let _ = std::fs::remove_dir_all(&dir);
}
