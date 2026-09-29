//! End-to-end: canned HTTP server (std, no deps) + full engine run.
//! Proves: robots enforcement, link following, extraction, dedup, honest
//! backoff on refusals (no hammering), JSONL sink output.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use harvest::engine;
use harvest::Config;

const PAGES: &[(&str, &str)] = &[
    (
        "/index.html",
        "<html><head><title>i</title></head><body><h1>Index Page</h1>\
         <p class=\"price\">$9.00</p><li class=\"tag\">alpha</li><li class=\"tag\">beta</li>\
         <a class=\"product\" href=\"/product/1\">one</a>\
         <a class=\"product\" href=\"/product/2\">two-identical</a>\
         <a href=\"/private/secret\">no</a></body></html>",
    ),
    (
        "/product/1",
        "<html><body><h1>Product One</h1><p class=\"price\">$19.99</p>\
         <a href=\"/index.html\">up</a><a href=\"/product/1\">self</a></body></html>",
    ),
    (
        "/product/2",
        // identical visible text to /product/1 => fingerprint dedup
        "<html><body><h1>Product   One</h1><p class=\"price\">$19.99</p>\
         <a href=\"/index.html\">up</a><a href=\"/product/1\">self</a></body></html>",
    ),
];

fn respond(stream: &mut TcpStream, path: &str, log: &[String]) -> std::io::Result<()> {
    let _ = log;
    if path == "/robots.txt" {
        let body = "User-agent: *\nDisallow: /private/\n";
        write!(
            stream,
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    } else if path == "/blocked/never" {
        let body = "forbidden";
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\nretry-after: 2\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    } else if path.starts_with("/private/") {
        let body = "secret";
        write!(
            stream,
            "HTTP/1.1 403 Forbidden\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    } else {
        let body = PAGES
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, b)| *b)
            .unwrap_or("<html><body>404</body></html>");
        let status = if PAGES.iter().any(|(p, _)| *p == path) {
            "200 OK"
        } else {
            "404 Not Found"
        };
        write!(
            stream,
            "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }?;
    Ok(())
}

/// Spawns the mock server; returns base URL and request log.
fn spawn_server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let log2 = log.clone();
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
            log2.lock().unwrap().push(path.clone());
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
            let _ = respond(&mut stream, &path, &[]);
            let _ = stream.flush();
        }
    });
    (format!("http://127.0.0.1:{port}"), log)
}

fn base_config(port_url: &str, out: &std::path::Path) -> Config {
    let cfg_json = serde_json::json!({
        "name": "e2e",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_retries": 3,
        "retry_base_ms": 5,
        "max_pages": 25,
        "default_policy": { "requests_per_sec": 5000.0, "burst": 100, "min_interval_ms": 0 },
        "block_backoff_secs": 2,
        "block_breaker_count": 3,
        "metrics_addr": "127.0.0.1:0",
        "targets": [{
            "id": "demo",
            "start_url": format!("{port_url}/index.html"),
            "max_depth": 2,
            "follow_links": true,
            "max_pages": 10,
            "extract": {
                "title": { "css": "h1" },
                "price": { "css": "p.price" },
                "tags": { "css": "li.tag", "all": true },
                "link": { "css": "a.product", "attr": "href" }
            }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    let mut cfg: Config = serde_json::from_value(cfg_json).unwrap();
    cfg.validate().unwrap();
    cfg
}

#[tokio::test]
async fn mock_server_smoke() {
    let (url, _log) = spawn_server();
    let host = url.trim_start_matches("http://").to_string();
    let mut sock = TcpStream::connect(&host).expect("connect");
    sock.set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    sock.set_write_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    sock.write_all(format!("GET /index.html HTTP/1.1\r\nhost: {host}\r\n\r\n").as_bytes())
        .unwrap();
    let mut got = String::new();
    let mut buf = [0u8; 4096];
    loop {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => got.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(e) => panic!("read failed: {e}; got so far: {got:?}"),
        }
    }
    assert!(
        got.starts_with("HTTP/1.1 200 OK"),
        "unexpected response: {got}"
    );
    assert!(got.contains("Index Page"), "{got}");
}

#[tokio::test]
async fn full_run_collects_dedups_and_respects_robots() {
    let (url, log) = spawn_server();
    let dir = std::env::temp_dir().join(format!("harvest-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.join("results.jsonl");

    let cfg = base_config(&url, &out);
    let summary = engine::run(cfg).await.expect("run");

    assert_eq!(
        summary.records, 2,
        "dedup must drop /product/2: {summary:?}"
    );
    assert_eq!(summary.written, 2);
    assert_eq!(summary.deduped, 1);
    assert!(
        summary.robots_denied >= 1,
        "private/secret must be denied: {summary:?}"
    );

    let log = log.lock().unwrap();
    assert!(
        !log.iter().any(|p| p.starts_with("/private/")),
        "must not even request denied URL: {log:?}"
    );
    assert!(log.iter().any(|p| p == "/robots.txt"));
    let fetches_of = |p: &str| log.iter().filter(|x| x.as_str() == p).count();
    assert_eq!(
        fetches_of("/index.html"),
        1,
        "exactly one fetch per page (budget ok)"
    );
    assert_eq!(fetches_of("/product/1"), 1);

    let lines: Vec<String> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 2);
    let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(v["fields"]["title"], "Index Page");
    assert_eq!(v["fields"]["tags"][0], "alpha");
    assert_eq!(v["fields"]["link"], "/product/1");
    assert_eq!(v["url"], format!("{url}/index.html"));
    let w: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert_eq!(w["fields"]["title"], "Product One");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn serve_loop_records_history_and_stops_after_runs_cap() {
    use std::sync::atomic::{AtomicBool, Ordering as O};
    use std::sync::Arc;

    let (url, log) = spawn_server();
    let dir = std::env::temp_dir().join(format!(
        "harvest-serve-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let hist = dir.join("runs.jsonl");

    let base = r#"{
        "name": "serve-demo",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_retries": 0,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 10000.0, "burst": 50, "min_interval_ms": 0 },
        "schedule": { "every_secs": 0, "runs": 3, "history_path": "__HIST__" },
        "targets": [{ "id": "t", "start_url": "__URL__/index.html", "follow_links": false,
          "extract": { "title": { "css": "h1" } } }],
        "output": { "kind": "null" }
    }"#
    .replace("__HIST__", &hist.to_string_lossy().replace('\\', "/"))
    .replace("__URL__", &url);

    let mut cfg: harvest::Config = serde_json::from_str(&base).unwrap();
    cfg.validate().unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let done = harvest::runner::serve(cfg, stop.clone()).await.unwrap();
    assert_eq!(done, 3, "runs cap must be honored");
    assert!(!stop.load(O::Relaxed));

    let txt = std::fs::read_to_string(&hist).unwrap();
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines.len(), 3, "one history row per scheduled run");
    for l in &lines {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(
            v["summary"]["records"], 1,
            "each run collected the index page"
        );
        assert_eq!(
            v["config_sha256"].as_str().unwrap().len(),
            64,
            "policy attestation present"
        );
    }
    // all three runs hit the source: 3 × (robots + page) = 6 requests
    let total = log.lock().unwrap().len();
    assert!(total >= 6, "expected >=6 server hits, got {total}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn http_sink_posts_records() {
    // ingest server: accepts POSTed JSON arrays, counts records received.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let received = Arc::new(AtomicUsize::new(0));
    let recv = received.clone();
    let sink_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let sink_port = sink_listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in sink_listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            let mut clen = 0usize;
            loop {
                let mut h = String::new();
                if reader.read_line(&mut h).is_err() || h == "\n" || h == "\r\n" {
                    break;
                }
                if h.to_lowercase().starts_with("content-length:") {
                    clen = h.split_once(':').unwrap().1.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; clen];
            let _ = reader.read_exact(&mut body);
            if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&body) {
                recv.fetch_add(v.as_array().map_or(0, |a| a.len()), Ordering::SeqCst);
            }
            let ok = "{\"ok\":true}";
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{ok}",
                    ok.len()
                )
                .as_bytes(),
            );
            let _ = stream.flush();
        }
    });

    let (url, _log) = spawn_server();
    let raw = serde_json::json!({
        "name": "sink",
        "user_agent": "harvest-test/0.1 (contact: e@x)",
        "metrics_addr": "",
        "max_retries": 0,
        "default_policy": { "requests_per_sec": 10000.0, "burst": 10, "min_interval_ms": 0 },
        "targets": [{
            "id": "t",
            "start_url": format!("{url}/index.html"),
            "follow_links": false,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "http", "url": format!("http://127.0.0.1:{sink_port}/ingest"), "batch_max": 100 }
    });
    let mut cfg: harvest::Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    let summary = harvest::engine::run(cfg).await.unwrap();
    assert_eq!(summary.written, 1);
    std::thread::sleep(std::time::Duration::from_millis(150)); // let sink thread settle
    assert_eq!(
        received.load(Ordering::SeqCst),
        1,
        "record must arrive via HTTP POST"
    );
}

#[tokio::test]
async fn refusals_are_honored_not_hammered() {
    let (url, log) = spawn_server();
    let dir = std::env::temp_dir().join(format!("harvest-e2e-blk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.join("results.jsonl");

    let mut cfg = base_config(&url, &out);
    // Retarget to a page that always 403s.
    let target = &mut cfg.targets[0];
    target.start_url = format!("{url}/blocked/never");
    target.follow_links = false;
    target.max_depth = 0;
    target.extract.clear();
    target.extract.insert(
        "title".into(),
        serde_json::from_str(r#"{"css":"h1"}"#).unwrap(),
    );
    let summary = engine::run(cfg).await.expect("run");

    assert_eq!(summary.records, 0, "403 yields no data");
    assert!(summary.blocked >= 1);
    let log = log.lock().unwrap();
    let hits = log
        .iter()
        .filter(|p| p.as_str() == "/blocked/never")
        .count();
    assert_eq!(
        hits, 1,
        "blocked responses must not be retried (Retry-After honored instead): got {hits}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn graceful_drain_stops_mid_run_without_losing_results() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    // Slow mock: a hub linking 24 unique pages, each served with 40ms delay.
    let hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits2 = hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            let body = match path.as_str() {
                "/robots.txt" => "User-agent: *\nDisallow:\n".to_string(),
                "/hub.html" => format!(
                    "<html><body><h1>Hub</h1>{}</body></html>",
                    (0..24)
                        .map(|i| format!("<a href=\"/p/{i}\">p{i}</a>"))
                        .collect::<String>()
                ),
                _ => {
                    std::thread::sleep(std::time::Duration::from_millis(40));
                    let n = hits2.fetch_add(1, Ordering::SeqCst);
                    format!("<html><body><h1>Page {n}</h1></body></html>")
                }
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-drain-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let out = dir.join("partial.jsonl");

    let raw = serde_json::json!({
        "name": "drain",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_retries": 0,
        "max_pages": 25,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 5000.0, "burst": 50, "min_interval_ms": 0 },
        "targets": [{
            "id": "slow",
            "start_url": format!("http://127.0.0.1:{port}/hub.html"),
            "follow_links": true,
            "max_depth": 1,
            "max_pages": 24,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    let cfg: harvest::Config = serde_json::from_value(raw).unwrap();

    // Stop as soon as a handful of slow pages are in; drain must land them.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let (hits3, stop3) = (hits.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                if hits3.load(Ordering::SeqCst) >= 4 {
                    stop3.store(true, std::sync::atomic::Ordering::Relaxed);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        });
    }

    let summary = harvest::engine::run_with(
        std::sync::Arc::new(cfg),
        harvest::engine::RunCtx {
            metrics: std::sync::Arc::new(harvest::metrics::Metrics::default()),
            pool: std::sync::Arc::new(harvest::proxy::ProxyPool::new(&[])),
            runtime: harvest::runtime::RuntimeSlot::default(),
        },
        Some(stop.clone()),
    )
    .await
    .expect("drained run returns Ok with partial results");

    assert!(
        summary.written >= 4,
        "results already in flight must be kept: {summary:?}"
    );
    assert!(
        summary.written < 24,
        "drain must abandon the queue, not finish it: {summary:?}"
    );
    let file_lines = std::fs::read_to_string(&out).unwrap().lines().count();
    assert_eq!(
        file_lines, summary.written as usize,
        "flush must persist everything counted"
    );
    let served = hits.load(Ordering::SeqCst);
    assert!(
        served <= summary.written as usize + 4,
        "no hammering after stop: served={served}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn dlq_records_failures_and_retry_recovers() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // /flaky answers 500 for its first 2 requests (retry+fail), then 200.
    let hits = Arc::new(AtomicUsize::new(0));
    let hits2 = hits.clone();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            let n = hits2.fetch_add(1, Ordering::SeqCst);
            let (status, body) = if path == "/robots.txt" {
                ("200 OK", "User-agent: *\nDisallow:\n".to_string())
            } else if path == "/flaky" && n < 2 {
                ("500 Internal Server Error", "nope".to_string())
            } else {
                (
                    "200 OK",
                    "<html><body><h1>Recovered</h1></body></html>".to_string(),
                )
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-dlq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dlq = dir.join("dlq.jsonl");
    let out = dir.join("res.jsonl");

    let mk = || {
        let raw = serde_json::json!({
            "name": "dlqtest",
            "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
            "respect_robots": true,
            "concurrency": 1,
            "max_retries": 0,
            "metrics_addr": "",
            "dlq_path": dlq.to_str().unwrap(),
            "dlq_max_attempts": 3,
            "default_policy": { "requests_per_sec": 1000.0, "burst": 10, "min_interval_ms": 0 },
            "targets": [{ "id": "t", "start_url": format!("http://127.0.0.1:{port}/flaky"),
                "extract": { "title": { "css": "h1" } } }],
            "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
        });
        let mut cfg: harvest::Config = serde_json::from_value(raw).unwrap();
        cfg.validate().unwrap();
        cfg
    };

    // run 1: transport-fails (500 x1 with max_retries 0) => DLQ gets the URL,
    // target summary records a failure, nothing written.
    let s1 = harvest::engine::run(mk()).await.unwrap();
    assert_eq!(s1.written, 0);
    assert_eq!(
        s1.per_target.get("t").unwrap().failed,
        1,
        "{:?}",
        s1.per_target
    );
    let d1 = std::fs::read_to_string(&dlq).unwrap();
    assert_eq!(d1.lines().count(), 1, "one dlq entry: {d1}");
    let v: serde_json::Value = serde_json::from_str(d1.lines().next().unwrap()).unwrap();
    assert!(v["url"].as_str().unwrap().ends_with("/flaky"), "{v}");
    assert!(v["reason"].as_str().unwrap().starts_with("failed:"), "{v}");
    assert_eq!(v["attempts"], 1);

    // run 2 with retry: flaky now answers 200 => recovered, DLQ cleared.
    let s2 = harvest::engine::run_with_stop(mk(), None, true, false)
        .await
        .unwrap();
    assert_eq!(s2.written, 1, "retry must collect the page: {s2:?}");
    let d2 = std::fs::read_to_string(&dlq).unwrap();
    assert_eq!(
        d2.trim().lines().count(),
        0,
        "recovered entry must leave the DLQ: {d2:?}"
    );
    let titles: Vec<String> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["fields"]["title"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(titles, vec!["Recovered".to_string()]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Sitemap-driven seeding through a REAL socket server: index → child sitemaps
/// → pages, with robots, include/exclude and same-site rules enforced, and the
/// start-URL overlap deduped via `visited`.
#[tokio::test]
async fn sitemap_seeding_respects_robots_filters_and_overlaps() {
    use std::collections::HashMap;
    let counts: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let routes: HashMap<&str, (&str, &str)> = [
        ("/robots.txt", ("text/plain", "User-agent: *\nDisallow: /secret\n")),
        ("/sitemap.xml", ("application/xml", "<urlset-irrelevant><sitemapindex><sitemap><loc>/sm-products.xml</loc></sitemap><sitemap><loc>http://127.0.0.1:999/sm-news.xml</loc></sitemap></sitemapindex>")),
        ("/sm-products.xml", ("application/xml", "<urlset><url><loc>http://127.0.0.1:PORT/product/1</loc></url><url><loc>/product/2</loc></url><url><loc>http://127.0.0.1:PORT/secret/leak</loc></url><url><loc>http://off.example/nope</loc></url></urlset>")),
        ("/sm-news.xml", ("application/xml", "<urlset><url><loc>http://127.0.0.1:999/news/a</loc></url></urlset>")),
        ("/index.html", ("text/html", "<html><body><h1>Index</h1><a href=\"/product/1\">dup</a></body></html>")),
        ("/product/1", ("text/html", "<html><body><h1>P1</h1></body></html>")),
        ("/product/2", ("text/html", "<html><body><h1>P2</h1></body></html>")),
        ("/secret/leak", ("text/html", "<html><body><h1>LEAK</h1></body></html>")),
    ]
    .into_iter()
    .collect();
    let routes = Arc::new(routes);

    let routes2 = routes.clone();
    let counts2 = counts.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            *counts2.lock().unwrap().entry(path.clone()).or_default() += 1;
            let body0 = routes2.get(path.as_str()).map(|(ct, b)| (*ct, *b));
            let Some((ctype, body)) = body0 else {
                let b = "gone";
                let _ = write!(
                    stream,
                    "HTTP/1.1 404 NF\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}",
                    b.len()
                );
                let _ = stream.flush();
                continue;
            };
            let body = body.replace("PORT", &port.to_string());
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-sm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("res.jsonl");

    let raw = serde_json::json!({
        "name": "smoke-sitemap",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 3,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 1000.0, "burst": 50, "min_interval_ms": 0 },
        "targets": [{
            "id": "site",
            "start_url": format!("http://127.0.0.1:{port}/index.html"),
            "sitemap": true,
            "follow_links": true,
            "max_depth": 1,
            "include": ["/product/", "/news/"],
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    let mut cfg: Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    let s = engine::run(cfg).await.unwrap();

    // index (start) + product/1 + product/2 (sitemap; product/1 deduped vs link).
    // news/a lives on the *other port* (999) so it never resolves — the point:
    // off-site and cross-port URLs are dropped before any queueing.
    let mut urls: Vec<String> = std::fs::read_to_string(&out)
        .unwrap_or_default()
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["url"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(s.written, 3, "index + two products: {s:?}");
    assert_eq!(
        s.robots_denied, 0,
        "secret was pre-filtered, not fetched-then-denied"
    );

    let c = counts.lock().unwrap();
    assert_eq!(
        *c.get("/secret/leak").unwrap_or(&0),
        0,
        "robots-disallowed URL must never be requested"
    );
    assert_eq!(
        *c.get("/sitemap.xml").unwrap_or(&0),
        1,
        "index doc fetched once"
    );
    assert_eq!(
        *c.get("/sm-products.xml").unwrap_or(&0),
        1,
        "child doc fetched once"
    );
    assert_eq!(
        c.get("/sm-news.xml").copied().unwrap_or(0),
        0,
        "cross-port child never fetched (404-proof: it is simply absent)"
    );

    urls.sort();
    urls.dedup();
    assert_eq!(
        urls,
        vec![
            format!("http://127.0.0.1:{port}/index.html"),
            format!("http://127.0.0.1:{port}/product/1"),
            format!("http://127.0.0.1:{port}/product/2"),
        ]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// JSON mode: typed extraction from an API body (numbers stay numbers,
/// wildcards collect arrays, missing paths are null — not empty strings).
#[tokio::test]
async fn json_api_mode_extracts_typed_fields() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            let (ctype, body) = if path == "/robots.txt" {
                ("text/plain", "User-agent: *\nDisallow:\n".to_string())
            } else if path == "/api/items" {
                (
                    "application/json",
                    r#"{"data":{"items":[{"id":1,"name":"Alpha","price":19.99,"tags":["x","y"]},{"id":2,"name":"Beta","price":5,"tags":[]}],"next":null,"ok":true}}"#.to_string(),
                )
            } else {
                (
                    "text/html",
                    "<html><body><h1>404</h1></body></html>".to_string(),
                )
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-json-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("rec.jsonl");

    let raw = serde_json::json!({
        "name": "smoke-json",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 1,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 1000.0, "burst": 10, "min_interval_ms": 0 },
        "targets": [{
            "id": "api",
            "mode": "json",
            "start_url": format!("http://127.0.0.1:{port}/api/items"),
            "follow_links": false,
            "extract": {
                "names":  { "path": "$.data.items[*].name", "all": true },
                "price0": { "path": "$.data.items[0].price" },
                "id1":    { "path": "$.data.items[1].id" },
                "ok":     { "path": "$.data.ok" },
                "next":   { "path": "$.data.next" },
                "absent": { "path": "$.nope[7]" }
            }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    let mut cfg: Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    let s = engine::run(cfg).await.unwrap();
    assert_eq!(s.written, 1, "{s:?}");

    let rec: serde_json::Value = serde_json::from_str(
        std::fs::read_to_string(&out)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    let f = &rec["fields"];
    assert_eq!(f["names"], serde_json::json!(["Alpha", "Beta"]));
    assert_eq!(f["price0"], serde_json::json!(19.99), "number, not string");
    assert_eq!(f["id1"], serde_json::json!(2));
    assert_eq!(f["ok"], serde_json::json!(true));
    assert_eq!(
        f["next"],
        serde_json::Value::Null,
        "explicit null in doc => null"
    );
    assert_eq!(f["absent"], serde_json::Value::Null, "missing path => null");

    // And validation must reject mixed-mode configs loudly.
    let bad = serde_json::json!({
        "name": "x", "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "targets": [{ "id": "t", "mode": "json", "start_url": format!("http://127.0.0.1:{port}/api/items"),
            "follow_links": false, "extract": { "title": { "css": "h1" } } }]
    });
    let mut bad: Config = serde_json::from_value(bad).unwrap();
    let err = bad.validate().unwrap_err().to_string();
    assert!(err.contains("css/attr") && err.contains("path"), "{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// robots.txt `Crawl-delay: 1` must actually pace the crawl: with the config
/// policy effectively unlimited (rps 1e9), 4 pages require ~3 host-minimum
/// gaps of ~1 s, i.e. the run cannot finish in milliseconds.
#[tokio::test]
async fn crawl_delay_from_robots_paces_the_engine() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            let (ctype, body) = if path == "/robots.txt" {
                (
                    "text/plain",
                    "User-agent: *\nDisallow:\nCrawl-delay: 1\n".to_string(),
                )
            } else if path == "/index.html" {
                (
                    "text/html",
                    (0..3)
                        .map(|i| format!("<a href=\"/p/{i}\">p{i}</a>"))
                        .collect::<Vec<_>>()
                        .join(""),
                )
            } else {
                (
                    "text/html",
                    format!("<html><body><h1>{path}</h1></body></html>"),
                )
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-delay-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let raw = serde_json::json!({
        "name": "delay-smoke",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 3,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 1e9, "burst": 100, "min_interval_ms": 0 },
        "targets": [{ "id": "d", "start_url": format!("http://127.0.0.1:{port}/index.html"),
            "follow_links": true, "max_depth": 1, "extract": { "title": { "css": "h1" } } }],
        "output": { "kind": "jsonl", "path": dir.join("o.jsonl").to_str().unwrap() }
    });
    let mut cfg: Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();

    let t0 = std::time::Instant::now();
    let s = engine::run(cfg).await.unwrap();
    let dur = t0.elapsed();
    assert_eq!(s.written, 4, "{s:?}");
    // index fetched, then 3 paced pages: >= 2 x ~1s of enforced gaps with
    // concurrency>1 (two may share the same sleep window).
    assert!(
        dur.as_millis() >= 1800,
        "Crawl-delay: 1s must serialize page fetches, run took {dur:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reliability centerpiece: a crawl interrupted mid-flight (SIGINT-style
/// stop flag — exactly what the CLI installs) leaves a checkpoint on disk;
/// `--resume` finishes the job with ZERO refetching, and a completed run
/// removes the file. Also proves the policy-drift guard refuses a resume
/// under changed config rather than silently violating the crawl's rules.
#[tokio::test]
async fn checkpoint_resume_finishes_crawl_with_zero_refetch() {
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    const PRODUCTS: usize = 8; // index + 8 pages = 9 records total
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let counts: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    let counts2 = counts.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            *counts2.lock().unwrap().entry(path.clone()).or_default() += 1;
            let body = if path == "/robots.txt" {
                "User-agent: *\nDisallow:\n".to_string()
            } else if path == "/index.html" {
                (0..PRODUCTS)
                    .map(|i| format!("<a href=\"/p/{i}\">p{i}</a>"))
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                format!("<html><body><h1>Page {path}</h1></body></html>")
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-ckpt-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("res.jsonl");
    let ckpt = dir.join("state.ckpt");

    let mk = || {
        let raw = serde_json::json!({
            "name": "ckpt-e2e",
            "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
            "respect_robots": true,
            "concurrency": 1,          // deterministic ordering for the test
            "max_pages": 200,
            "metrics_addr": "",
            "ckpt_path": ckpt.to_str().unwrap(),
            "ckpt_every_pages": 2,
            "ckpt_every_secs": 3600,   // exercise the count gate + cancel write only
            "default_policy": { "requests_per_sec": 1e9, "burst": 100, "min_interval_ms": 150 },
            "targets": [{ "id": "s", "start_url": format!("http://127.0.0.1:{port}/index.html"),
                "follow_links": true, "max_depth": 1,
                "extract": { "title": { "css": "h1" } } }],
            "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
        });
        let mut cfg: Config = serde_json::from_value(raw).unwrap();
        cfg.validate().unwrap();
        cfg
    };

    // ---- run 1: interrupt after 3 records --------------------------------
    let stop = Arc::new(AtomicBool::new(false));
    let stop_task = stop.clone();
    let out_task = out.clone();
    let watcher = tokio::spawn(async move {
        loop {
            let n = std::fs::read_to_string(&out_task)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if n >= 3 {
                stop_task.store(true, std::sync::atomic::Ordering::Relaxed);
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    });
    let s1 = engine::run_with_stop(mk(), Some(stop), false, false)
        .await
        .unwrap();
    watcher.await.unwrap();
    // Drain semantics: a worker finishes the job it already popped, so the
    // cut is 3 pages plus at most one in-flight — the whole test below is
    // written relative to wherever the boundary actually landed.
    let w1 = s1.written;
    assert!((3..=4).contains(&w1), "interrupted at 3-4 pages: {s1:?}");
    assert!(
        s1.ckpt_writes >= 1,
        "final checkpoint write must be counted: {s1:?}"
    );
    assert!(ckpt.exists(), "interrupted run must leave a checkpoint");

    let ck_raw = std::fs::read_to_string(&ckpt).unwrap();
    let ck: serde_json::Value = serde_json::from_str(&ck_raw).unwrap();
    assert_eq!(ck["v"], 1);
    assert_eq!(ck["run"], "ckpt-e2e");
    assert_eq!(ck["global_pages"].as_u64().unwrap(), w1);
    assert_eq!(
        ck["pending"].as_array().unwrap().len(),
        (9 - w1 as usize),
        "queued-but-unfetched pages survive"
    );
    assert_eq!(
        ck["targets"][0]["visited"].as_array().unwrap().len(),
        9,
        "index + all 8 links already marked"
    );
    assert_eq!(ck["targets"][0]["written"].as_u64().unwrap(), w1);

    let pages1: Vec<String> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["url"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(pages1.len() as u64, w1);

    // ---- policy drift: resuming under a changed config must be REFUSED ----
    let mut drift = mk();
    drift.max_pages = 999;
    let err = engine::run_with_stop(drift, None, false, true)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("config changed since"),
        "drift guard must fire: {err}"
    );
    assert!(
        ckpt.exists(),
        "a refused resume must not consume the checkpoint"
    );

    // ---- run 2: resume picks up exactly where run 1 stopped ---------------
    let s2 = engine::run_with_stop(mk(), None, false, true)
        .await
        .unwrap();
    assert_eq!(
        s2.written,
        9 - w1,
        "resume finishes exactly what is left: {s2:?} (run1 did {w1})"
    );

    // Zero refetch proof: across BOTH runs every page was requested once.
    // NOTE: guard must be scoped — the mock server locks the same map per
    // request, so holding it across a fetch path starves the crawler.
    {
        let c = counts.lock().unwrap();
        assert_eq!(*c.get("/index.html").unwrap(), 1, "index never refetched");
        for i in 0..PRODUCTS {
            assert_eq!(
                *c.get(&format!("/p/{i}")).unwrap_or(&0),
                1,
                "/p/{i} fetched exactly once across runs"
            );
        }
    }
    // Full, unique result set in the sink (append-across-runs continuity).
    let mut all: Vec<String> = std::fs::read_to_string(&out)
        .unwrap()
        .lines()
        .map(|l| {
            serde_json::from_str::<serde_json::Value>(l).unwrap()["url"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    all.sort();
    let uniq = all.len();
    all.dedup();
    assert_eq!(
        all.len(),
        uniq,
        "no duplicate records across the resume boundary"
    );
    assert_eq!(all.len(), 9, "index + 8 products");

    // ---- complete run deletes the checkpoint; resume becomes a fresh no-op --
    assert!(!ckpt.exists(), "checkpoint removed after a complete run");
    let s3 = engine::run_with_stop(mk(), None, false, true)
        .await
        .unwrap();
    assert_eq!(
        s3.written, 9,
        "no file => warned fresh start, full crawl under dedup budget: {s3:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Incremental revalidation end-to-end against a live conditional-GET mock:
/// run 1 pays full fetches; run 2 (unchanged site) pays ONLY 304s — zero
/// records, zero bodies — yet still discovers and fetches a page the site
/// added between runs, because discovery replays the cached link graph.
/// Third run after another change: the changed doc refetches, others 304.
#[tokio::test]
async fn revalidation_skips_bodies_but_not_the_crawl() {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Generation: 1 = {index->[p1,p2]}, 2 = index also links p3, p2 changed.
    let gen = Arc::new(AtomicUsize::new(1));
    let stats: Arc<Mutex<HashMap<String, [usize; 2]>>> = Arc::new(Mutex::new(HashMap::new())); // [requests, 304s]
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    {
        let gen = gen.clone();
        let stats = stats.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut stream) = conn else { continue };
                let Ok(clone) = stream.try_clone() else {
                    continue;
                };
                let mut reader = BufReader::new(clone);
                let mut req = String::new();
                if reader.read_line(&mut req).is_err() {
                    continue;
                }
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let mut inm = String::new();
                loop {
                    let mut hdr = String::new();
                    if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                        break;
                    }
                    let l = hdr.to_ascii_lowercase();
                    if let Some(rest) = l.strip_prefix("if-none-match:") {
                        inm = rest.trim().to_string();
                    }
                }
                let g = gen.load(Ordering::SeqCst);
                let body = match path.as_str() {
                    "/robots.txt" => "User-agent: *\nDisallow:\n".to_string(),
                    "/index.html" => format!(
                        "<html><body><h1>Index G{g}</h1><a href=\"/p1\">1</a><a href=\"/p2\">2</a>{}</body></html>",
                        if g >= 2 { "<a href=\"/p3\">3</a>" } else { "" }
                    ),
                    "/p1" => "<html><body><h1>P1 stable</h1></body></html>".to_string(),
                    // p2 *changes* in generation 2; its etag tracks content
                    "/p2" if g >= 2 => "<html><body><h1>P2 REVISED</h1></body></html>".to_string(),
                    "/p2" => "<html><body><h1>P2 stable</h1></body></html>".to_string(),
                    "/p3" => "<html><body><h1>P3 new</h1></body></html>".to_string(),
                    _ => "gone".to_string(),
                };
                let tag = {
                    use std::hash::{Hash, Hasher};
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    body.hash(&mut h);
                    format!("\"h{:x}\"", h.finish()) // etag tracks CONTENT only
                };
                {
                    let mut gs = stats.lock().unwrap();
                    let st = gs.entry(path.clone()).or_insert([0, 0]);
                    st[0] += 1;
                    if path != "/robots.txt" && inm == tag {
                        st[1] += 1;
                    }
                }
                if path != "/robots.txt" && inm == tag {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 304 Not Modified\r\nconnection: close\r\n\r\n"
                    );
                    let _ = stream.flush();
                    continue;
                }
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\netag: {tag}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
                let _ = stream.flush();
            }
        });
    }

    let dir = std::env::temp_dir().join(format!("harvest-reval-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("res.jsonl");
    let store = dir.join("validators.jsonl");

    let mk = || {
        let raw = serde_json::json!({
            "name": "reval-e2e",
            "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
            "respect_robots": true,
            "concurrency": 1,
            "metrics_addr": "",
            "revalidate_store": store.to_str().unwrap(),
            "default_policy": { "requests_per_sec": 1e9, "burst": 50, "min_interval_ms": 0 },
            "targets": [{ "id": "s", "start_url": format!("http://127.0.0.1:{port}/index.html"),
                "follow_links": true, "max_depth": 1, "extract": { "title": { "css": "h1" } } }],
            "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
        });
        let mut cfg: Config = serde_json::from_value(raw).unwrap();
        cfg.validate().unwrap();
        cfg
    };
    let snap = || stats.lock().unwrap().clone();
    let read = |f: &std::path::Path| -> Vec<serde_json::Value> {
        std::fs::read_to_string(f)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };

    // ---- gen 1, run A: cold, all 200 --------------------------------------
    let a = engine::run(mk()).await.unwrap();
    assert_eq!(a.written, 3, "index p1 p2: {a:?}");
    assert_eq!(a.revalidated, 0);
    let s = snap();
    assert_eq!(s["/index.html"], [1, 0]);
    assert!(store.exists(), "validators file written at end of run");

    // ---- gen 1, run B: nothing changed => pure 304 crawl, zero records ----
    let b = engine::run(mk()).await.unwrap();
    assert_eq!(b.written, 0, "304s must not produce records: {b:?}");
    assert_eq!(b.revalidated, 3, "all three docs revalidated: {b:?}");
    let s = snap();
    assert_eq!(s["/index.html"], [2, 1], "index: second visit was a 304");
    assert_eq!(
        s["/p1"],
        [2, 1],
        "p1 reached via CACHED LINK GRAPH despite index body never re-downloaded"
    );
    assert_eq!(read(&out).len(), 3, "sink untouched by revalidations");

    // ---- gen 2: site links a NEW page and revises p2 ------------------------
    gen.store(2, Ordering::SeqCst);
    let c = engine::run(mk()).await.unwrap();
    // index changed (etag moved) => refetch + discover p3; p2 refetch; p1 304
    assert_eq!(c.written, 3, "index', p2', p3: {c:?}");
    assert_eq!(c.revalidated, 1, "p1 untouched => exactly one 304: {c:?}");
    let s = snap();
    assert_eq!(s["/p1"], [3, 2], "p1: 200,304,304");
    assert_eq!(s["/p3"], [1, 0], "p3 discovered purely from index delta");
    let rows = read(&out);
    let titles: Vec<&str> = rows
        .iter()
        .map(|r| r["fields"]["title"].as_str().unwrap())
        .collect();
    assert!(
        titles.contains(&"P3 new") && titles.contains(&"P2 REVISED"),
        "{titles:?}"
    );
    // dedup: 'P1 stable' was fetched again in run B? No: run B didn't fetch p1 body.
    // The sink must hold exactly one row per page title across all runs.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in read(&out) {
        *counts
            .entry(r["fields"]["title"].as_str().unwrap().to_string())
            .or_default() += 1;
    }
    assert_eq!(counts["Index G1"], 1, "unchanged pages never rewritten");
    assert_eq!(
        counts["P1 stable"], 1,
        "304'd page never rewritten across three runs"
    );
    assert_eq!(counts["P3 new"], 1);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Politeness audit: the report must reconcile against what the server
// actually saw — that is its entire selling point. This mock counts every
// request it receives; the test asserts requests + robots fetches equal the
// server's own total, denied paths were never requested, and pacing/backoff
// time shows up in the ledger.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn audit_report_reconciles_with_server_log() {
    use std::collections::HashMap;
    use std::net::TcpListener;
    use std::sync::Mutex;

    let dir = std::env::temp_dir().join(format!("harvest-audit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hits: Arc<Mutex<HashMap<String, usize>>> = Arc::new(Mutex::new(HashMap::new()));
    let total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (h, t) = (hits.clone(), total.clone());
    std::thread::spawn(move || {
        let html = |b: &str| {
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                b.len(),
                b
            )
        };
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 8192];
            let Ok(n) = s.read(&mut buf) else { continue };
            let head = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or("")
                .to_string();
            if !head.starts_with("GET ") {
                // readiness probe / junk connection: never counted.
                let _ = s.write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n");
                continue;
            }
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            *h.lock().unwrap().entry(path.clone()).or_default() += 1;
            t.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let body = match path.as_str() {
                "/robots.txt" => {
                    let b = "User-agent: *\nCrawl-delay: 1\nDisallow: /private/\n";
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        b.len(),
                        b
                    )
                }
                "/" => html("<html><body><h1>Index</h1><a href=\"/p1\">1</a><a href=\"/p2\">2</a><a href=\"/blocked\">b</a><a href=\"/private/x\">x</a></body></html>"),
                "/p1" | "/p2" => html("<html><body><h1>Same</h1></body></html>"),
                "/blocked" => "HTTP/1.1 403 Forbidden\r\ncontent-type: text/html\r\ncontent-length: 0\r\nconnection: close\r\n\r\n".to_string(),
                _ => html("<html><body><h1>Private</h1></body></html>"),
            };
            let _ = s.write_all(body.as_bytes());
            let _ = s.flush();
        }
    });

    // make sure the accept loop is live before the engine (and its request
    // counters) starts — this box schedules threads slowly under test load.
    for _ in 0..400 {
        if std::net::TcpStream::connect(addr).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let audit_path = dir.join("audit.json");
    let out = dir.join("out.jsonl");
    let key_path = dir.join("attest.hex");
    std::fs::write(
        &key_path,
        format!(
            "{}\n",
            (0u8..32).map(|i| format!("{i:02x}")).collect::<String>()
        ),
    )
    .unwrap();
    let cfg_json = serde_json::json!({
        "name": "audit-e2e",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_retries": 0,
        "max_pages": 25,
        "metrics_addr": "127.0.0.1:0",
        "default_policy": { "requests_per_sec": 5000.0, "burst": 100, "min_interval_ms": 0 },
        "block_backoff_secs": 2,
        "audit_path": audit_path.to_str().unwrap(),
        "attest_key_path": key_path.to_str().unwrap(),
        "targets": [{
            "id": "w",
            "start_url": format!("http://{addr}/"),
            "max_depth": 2,
            "follow_links": true,
            "max_pages": 25,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "jsonl", "path": out.to_str().unwrap() }
    });
    let cfg: Config = serde_json::from_value(cfg_json).unwrap();
    let t0 = std::time::Instant::now();
    let summary = engine::run(cfg).await.unwrap();
    let wall = t0.elapsed();
    assert_eq!(
        summary.written, 2,
        "index + p1 (p2 deduped, 403+robots-denied excluded): {summary:?} server saw: {hits:?}"
    );

    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&audit_path).unwrap()).unwrap();
    let host = &report["hosts"]["127.0.0.1"];
    // GROUND TRUTH: server's own hit count == our claimed requests + robots fetches.
    let server_total = total.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        host["requests"].as_u64().unwrap() + host["robots"]["fetched"].as_u64().unwrap(),
        server_total as u64,
        "audit must reconcile with the server log: {report:#}"
    );
    assert_eq!(host["requests"].as_u64().unwrap(), 4, "{report:#}"); // /, /p1, /p2, /blocked
    assert_eq!(host["ok200"].as_u64().unwrap(), 3);
    assert_eq!(host["blocked"]["403"].as_u64().unwrap(), 1);
    assert_eq!(report["totals"]["records_written"].as_u64().unwrap(), 2);
    assert_eq!(
        host["robots"]["fetched"].as_u64().unwrap(),
        1,
        "robots cached, fetched once"
    );
    assert!(host["robots"]["present"].as_bool().unwrap());
    assert_eq!(host["robots"]["denials"].as_u64().unwrap(), 1);
    // The disallowed path must never have been requested at all.
    assert_eq!(
        hits.lock().unwrap().get("/private/x"),
        None,
        "robots-denied path was fetched!"
    );
    // Crawl-delay: 1s floor paced the page requests and the ledger shows it.
    assert!(
        wall >= std::time::Duration::from_millis(1800),
        "crawl-delay floor not honored: {wall:?}"
    );
    assert!(
        host["wait_sleeps"].as_u64().unwrap() >= 2,
        "paced waits not counted: {host:#}"
    );
    assert!(
        host["wait_slept_ms"].as_u64().unwrap() >= 1800,
        "sleep time not recorded: {host:#}"
    );
    // 403 -> configured backoff recorded.
    assert!(
        host["backoff_ms"].as_u64().unwrap() >= 1900,
        "403 backoff not recorded: {host:#}"
    );
    assert_eq!(host["breaker_abandons"].as_u64().unwrap(), 0);
    // Policy echo + run attestation link.
    assert_eq!(host["policy"]["requests_per_sec"].as_f64().unwrap(), 5000.0);
    assert_eq!(report["policy_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(report["run"].as_str().unwrap(), "audit-e2e");
    // Markdown renders the receipts.
    let md = harvest::audit::render_markdown(&report);
    assert!(
        md.contains("127.0.0.1") && md.contains("403×1") && md.contains("Totals"),
        "{md}"
    );

    // Attestation: the engine signed the receipt; a holder of the key can
    // prove it is untouched — and can prove the opposite after one byte.
    let sig = audit_path.with_file_name("audit.json.sig");
    assert!(
        sig.exists(),
        "audit.json.sig must be written next to the receipt"
    );
    let k = harvest::sign::Key::load(&key_path).unwrap();
    k.verify_file(&audit_path, &sig)
        .expect("fresh receipt verifies");
    let keep = std::fs::read(&audit_path).unwrap();
    let mut tampered = keep.clone();
    tampered[1] ^= 0x01; // flip one bit somewhere in the JSON
    std::fs::write(&audit_path, &tampered).unwrap();
    assert!(
        k.verify_file(&audit_path, &sig).is_err(),
        "single-bit edit must break attestation"
    );
    std::fs::write(&audit_path, &keep).unwrap();
    k.verify_file(&audit_path, &sig)
        .expect("restored receipt verifies again");
    let _ = std::fs::remove_dir_all(&dir);
}
