//! Realistic operational workflow: a live `harvest serve` (mock origin),
//! the admin API, alerts dedup via cooldown, the audit trail, the CLI,
//! bounded listings, and redaction — end to end, on the real binary.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// Tiny origin: `/index.html` fans out to 3 products; `/hook` counts alert
/// deliveries (the cooldown assertion reads this count — dedup must be real
/// at the wire level, not just in the ledger).
fn mock_base(hook_hits: Arc<AtomicUsize>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming().take(400) {
            let Ok(mut sock) = conn else { continue };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let _ = sock.set_write_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 16384];
            let n = sock.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            if path.starts_with("/hook") {
                hook_hits.fetch_add(1, Ordering::SeqCst);
                let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok");
            } else {
                let body = if path == "/index.html" {
                    "<html><body><h1>Index</h1><p class=price>$1.00</p>\
                     <a href=\"/product/1\">a</a><a href=\"/product/2\">b</a><a href=\"/product/3\">c</a>\
                     </body></html>"
                        .to_string()
                } else if let Some(rest) = path.strip_prefix("/product/") {
                    format!(
                        "<html><body><h1>P{rest}</h1><p class=price>${rest}.50</p></body></html>"
                    )
                } else {
                    String::new()
                };
                let r = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(r.as_bytes());
            }
        }
    });
    format!("http://{addr}")
}

fn free_addr() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    drop(l);
    a
}

fn http(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, String, Vec<(String, String)>) {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: x\r\n{auth}content-length: {}\r\n\r\n{body}",
        body.len()
    );
    sock.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = sock.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let mut lines = head.lines();
    let status: u16 = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let headers = lines
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        })
        .collect();
    (status, rest.to_string(), headers)
}

struct ServeProc {
    child: Child,
}

impl ServeProc {
    fn start(cfg_path: &Path) -> ServeProc {
        let child = Command::new(env!("CARGO_BIN_EXE_harvest"))
            .args(["serve", "--config"])
            .arg(cfg_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        ServeProc { child }
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    /// Block until `lines` history lines exist (i.e. that many scheduled
    /// runs have FINISHED — per-run metrics stream while crawling, so only
    /// the durable history marks run completion).
    fn wait_runs(&mut self, history: &Path, lines: usize) {
        let t0 = Instant::now();
        loop {
            assert!(self.alive(), "serve exited early waiting for {lines} runs");
            assert!(
                t0.elapsed() < Duration::from_secs(40),
                "history never reached {lines} runs"
            );
            let n = std::fs::read_to_string(history)
                .map(|s| s.lines().count())
                .unwrap_or(0);
            if n >= lines {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    fn wait_exit(&mut self) {
        let status = self.child.wait().expect("reap serve");
        assert!(status.success(), "serve exited with a failure");
    }
}

impl Drop for ServeProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_harvest"))
        .args(args)
        .output()
        .expect("spawn harvest");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const TOK: &str = "s3cr3t-ops-token-xyz";
const TOK_ENV: &str = "HARVEST_OPSAPI_TEST_TOKEN";

#[test]
fn live_workflow_serve_api_alerts_audit_cli() {
    let hook_hits = Arc::new(AtomicUsize::new(0));
    let base = mock_base(hook_hits.clone());
    let addr = free_addr();
    let dir = std::env::temp_dir().join(format!("harvest-opsapi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("out")).unwrap();
    std::env::set_var(TOK_ENV, TOK);
    let cfg_path = dir.join("cfg.json");
    std::fs::write(
        &cfg_path,
        json!({
            "name": "opsapi",
            "respect_robots": false,
            "concurrency": 2,
            "max_pages": 8,
            "request_timeout_secs": 4,
            "metrics_addr": addr,
            "admin_token": format!("${{{TOK_ENV}}}"),
            "schedule": {"every_secs": 2, "runs": 3, "history_path": dir.join("out/runs.jsonl")},
            "output": {"kind": "jsonl", "path": dir.join("out/records.jsonl")},
            "events_path": dir.join("out/events.jsonl"),
            "alerts": {
                "webhook_url": format!("{base}/hook"),
                "events": ["no_data"],
                "min_records": 9999,
                "cooldown_secs": 3600
            },
            "targets": [{
                "id": "products",
                "start_url": format!("{base}/index.html"),
                "follow_links": true,
                "max_depth": 1,
                "extract": {"title": {"css": "h1"}, "price": {"css": "p.price"}}
            }]
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();

    let mut proc = ServeProc::start(&cfg_path);
    // wait for the admin listener to come up before any live query
    {
        let t0 = Instant::now();
        loop {
            assert!(proc.alive(), "serve died before the admin came up");
            assert!(
                t0.elapsed() < Duration::from_secs(15),
                "admin never bound {addr}"
            );
            if TcpStream::connect(&addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    // Catch a MID-RUN snapshot first: the queue is only "live" while a run
    // is crawling (idle between scheduled runs => unavailable, by design).
    let t0 = Instant::now();
    let mut saw_live = false;
    let mut saw_idle = false;
    while !saw_live || !saw_idle {
        assert!(proc.alive(), "serve died during queue sampling");
        assert!(
            t0.elapsed() < Duration::from_secs(30),
            "never caught a live run"
        );
        let (st, q, _) = http(&addr, "GET", "/queue", Some(TOK), "");
        assert_eq!(st, 200);
        let qv: Value = serde_json::from_str(&q).unwrap();
        match qv["state"].as_str().unwrap() {
            "healthy" | "degraded" => {
                assert!(qv["stats"].is_object(), "live queue carries stats: {qv}");
                saw_live = true;
            }
            "unavailable" => saw_idle = true,
            other => panic!("unexpected queue state {other}: {qv}"),
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    // past the end of run #2: enough durable history for every live view
    proc.wait_runs(&dir.join("out/runs.jsonl"), 2);

    // ---- auth is fail-closed, probes are open ----
    let (st, _, _) = http(&addr, "GET", "/health", None, "");
    assert_eq!(st, 200, "/health stays auth-exempt");
    let (st, body, headers) = http(&addr, "GET", "/stats", None, "");
    assert_eq!(st, 401, "no token => denied: {body}");
    assert!(headers
        .iter()
        .any(|(k, v)| k == "www-authenticate" && v == "Bearer"));
    assert!(!body.contains(TOK), "denied body leaks nothing: {body}");
    // a bogus token is the same constant reply
    let (st, bogus_body, _) = http(&addr, "GET", "/stats", Some("wrong"), "");
    assert_eq!(st, 401);
    let err_of = |b: &str| {
        serde_json::from_str::<Value>(b)
            .ok()
            .map(|v| v["error"].to_string())
            .unwrap_or_default()
    };
    assert_eq!(
        err_of(&bogus_body),
        err_of(&body),
        "401 replies are content-constant modulo the correlation id"
    );

    // ---- correlation id: echo a good one, generate otherwise ----
    let (st, _, headers) = http(&addr, "GET", "/stats", Some(TOK), "");
    assert_eq!(st, 200);
    let corr = headers
        .iter()
        .find(|(k, _)| k == "x-request-id")
        .expect("generated correlation header")
        .1
        .clone();
    assert!(corr.starts_with("req-"), "generated id: {corr}");
    let (st404, b404, h404) = http(&addr, "GET", "/definitely-nope", Some(TOK), "");
    assert_eq!(st404, 404);
    assert!(b404.starts_with("see /"));
    assert!(h404.iter().any(|(k, _)| k == "x-request-id"));
    let raw = {
        let mut sock = TcpStream::connect(&addr).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.write_all(
            b"GET /stats HTTP/1.1\r\nhost: x\r\nauthorization: Bearer s3cr3t-ops-token-xyz\r\nx-request-id: trace-42\r\n\r\n",
        )
        .unwrap();
        let mut b = Vec::new();
        let _ = sock.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    };
    assert!(raw.contains("x-request-id: trace-42"), "explicit id echoed");
    // header-injection attempts in the id cannot ride through: an invalid
    // value is replaced with a generated one, and a folded continuation
    // line never becomes a response header
    let sneaky = {
        let mut sock = TcpStream::connect(&addr).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        sock.write_all(
            b"GET /stats HTTP/1.1\r\nhost: x\r\nauthorization: Bearer s3cr3t-ops-token-xyz\r\nx-request-id: evil value\r\nx-injected: yes\r\n\r\n",
        )
        .unwrap();
        let mut b = Vec::new();
        let _ = sock.read_to_end(&mut b);
        String::from_utf8_lossy(&b).into_owned()
    };
    let head_part = sneaky.split("\r\n\r\n").next().unwrap_or(&sneaky);
    assert!(
        !head_part.contains("evil value"),
        "invalid id must be replaced, not echoed"
    );
    assert!(
        !head_part.contains("x-injected"),
        "unknown request line must not become a response header"
    );
    assert!(
        head_part.contains("x-request-id: req-"),
        "generated fallback id: {head_part}"
    );

    // ---- live views ----
    let (st, diag, _) = http(&addr, "GET", "/diagnostics", Some(TOK), "");
    assert_eq!(st, 200);
    let d: Value = serde_json::from_str(&diag).unwrap();
    let names: Vec<&str> = d["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    for want in ["config", "runtime", "proxies", "history", "events_log"] {
        assert!(
            names.contains(&want),
            "diagnostics misses {want}: {names:?}"
        );
    }
    let (st, jobs, _) = http(&addr, "GET", "/jobs", Some(TOK), "");
    assert_eq!(st, 200);
    let j: Value = serde_json::from_str(&jobs).unwrap();
    assert_eq!(j["job"]["jobs"][0]["target"], "products", "{j}");
    let (st, prom, _) = http(&addr, "GET", "/metrics", Some(TOK), "");
    assert_eq!(st, 200);
    assert!(prom.contains("harvest_records_written"), "core series");
    assert!(
        prom.contains("harvest_api_requests_total{class=\"2xx\"}"),
        "API status-class series"
    );
    assert!(
        prom.contains("harvest_queue_depth") || prom.contains("harvest_jobs_active"),
        "live runtime series: {prom}"
    );
    let (st, stor, _) = http(&addr, "GET", "/storage", Some(TOK), "");
    assert_eq!(st, 200);
    let sv: Value = serde_json::from_str(&stor).unwrap();
    assert_eq!(sv["history"]["exists"], true, "{sv}");
    assert!(
        !stor.contains(&dir.to_string_lossy().to_string()),
        "raw temp path leaked: {sv}"
    );

    // ---- alerts: cooldown dedup measured at the WEBHOOK, not just state ----
    let (st, alerts, _) = http(&addr, "GET", "/alerts", Some(TOK), "");
    assert_eq!(st, 200);
    let av: Value = serde_json::from_str(&alerts).unwrap();
    let item = &av["items"][0];
    assert_eq!(item["event"], "no_data", "{av}");
    assert!(
        item["fires"].as_u64().unwrap() >= 2,
        "runs accumulate fires: {av}"
    );
    assert_eq!(
        item["notified"], 1,
        "cooldown lets exactly one through: {av}"
    );
    assert!(item["suppressed"].as_u64().unwrap() >= 1, "{av}");
    assert_eq!(
        hook_hits.load(Ordering::SeqCst),
        1,
        "one webhook hit for N fires"
    );

    // ---- operator actions: note + ack, both audited ----
    let (st, _, _) = http(
        &addr,
        "POST",
        "/ops/note",
        Some(TOK),
        "{\"note\":\"on-call acknowledged the surge\"}",
    );
    assert_eq!(st, 200, "note accepted");
    let (st, ack, _) = http(&addr, "POST", "/alerts/ack?event=no_data", Some(TOK), "");
    assert_eq!(st, 200);
    assert!(ack.contains("\"acked\":1"), "{ack}");
    let (st, ops, _) = http(&addr, "GET", "/ops?limit=50", Some(TOK), "");
    assert_eq!(st, 200);
    let ov: Value = serde_json::from_str(&ops).unwrap();
    let kinds: Vec<&str> = ov["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"operator.note") && kinds.contains(&"alerts.ack"),
        "{ov}"
    );
    assert!(
        kinds.contains(&"run_start") && kinds.contains(&"run_end"),
        "run lifecycle ops events: {ov}"
    );
    let (st, audit, _) = http(&addr, "GET", "/audit", Some(TOK), "");
    assert_eq!(st, 200);
    let av2: Value = serde_json::from_str(&audit).unwrap();
    assert_eq!(
        av2["items"][0]["action"], "alerts.ack",
        "newest first: {av2}"
    );
    assert!(av2["items"][0]["corr"]
        .as_str()
        .unwrap()
        .starts_with("req-"));
    // a GET on a mutating route is 405, and oversized POST bodies are 413
    let (st, _, _) = http(&addr, "GET", "/alerts/ack?event=no_data", Some(TOK), "");
    assert_eq!(st, 405);
    let big = "x".repeat(9000);
    let (st, _, _) = http(
        &addr,
        "POST",
        "/ops/note",
        Some(TOK),
        &format!("{{\"note\":\"{big}\"}}"),
    );
    assert_eq!(st, 413, "body cap enforced");

    // ---- /config redaction while live ----
    let (st, cfgv, _) = http(&addr, "GET", "/config", Some(TOK), "");
    assert_eq!(st, 200);
    assert!(!cfgv.contains(TOK), "live /config leaks the admin token!");
    let cv: Value = serde_json::from_str(&cfgv).unwrap();
    assert_eq!(cv["admin_token"], "<redacted>");
    assert_eq!(cv["_sources"]["admin_token"], format!("env:{TOK_ENV}"));
    assert_eq!(cv["validation"], "ok");

    // ---- `harvest health` against the live server: exit 0, target reported
    let (rc, out, err) = cli(&["health", "--config", cfg_path.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "healthy", "{v}");
    assert_eq!(v["target"], addr);

    // ---- let serve finish run 3 and exit cleanly (runs=3) ----
    proc.wait_exit();
    assert_eq!(
        hook_hits.load(Ordering::SeqCst),
        1,
        "no new deliveries after ack"
    );

    // ---- durable records ----
    let hist = std::fs::read_to_string(dir.join("out/runs.jsonl")).unwrap();
    assert_eq!(hist.lines().count(), 3, "scheduled runs recorded");
    for line in hist.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["status"], "ok", "{v}");
        assert!(v["summary"]["written"].as_u64().unwrap() >= 3);
    }
    let recs = std::fs::read_to_string(dir.join("out/records.jsonl")).unwrap();
    assert!(recs.lines().count() >= 9, ">=3 records per run x 3 runs");
    let evtext = std::fs::read_to_string(dir.join("out/events.jsonl")).unwrap();
    assert!(
        evtext.contains("sink_write_completed"),
        "crawl events in the log"
    );

    // ---- CLI reads the same durable state; offline first ----
    let (rc, out, err) = cli(&[
        "runs",
        "--config",
        cfg_path.to_str().unwrap(),
        "--limit",
        "5",
        "--json",
    ]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["window"], 3);
    assert_eq!(v["items"].as_array().unwrap().len(), 3);
    assert!(v["items"][0]["status"] == "ok");
    let (rc, out, _) = cli(&[
        "runs",
        "--config",
        cfg_path.to_str().unwrap(),
        "--status",
        "error",
        "--jsonl",
    ]);
    assert_eq!(rc, 0);
    assert_eq!(
        out.trim().lines().count(),
        0,
        "error filter on an all-ok file"
    );
    let (rc, out, err) = cli(&["status", "--config", cfg_path.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["run"], "opsapi");
    assert_eq!(v["last_run"]["status"], "ok");
    let (rc, out, _) = cli(&[
        "config",
        "inspect",
        "--config",
        cfg_path.to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(rc, 0);
    assert!(!out.contains(TOK), "CLI leaks token");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["admin_token"], "<redacted>");
    assert_eq!(v["_sources"]["admin_token"], format!("env:{TOK_ENV}"));
    let (rc, out, _) = cli(&[
        "config",
        "explain",
        "--config",
        cfg_path.to_str().unwrap(),
        "--field",
        "alerts.",
        "--json",
    ]);
    assert_eq!(rc, 0);
    let rows: Value = serde_json::from_str(&out).unwrap();
    let knobs = rows["knobs"].as_array().unwrap();
    assert!(!knobs.is_empty());
    assert!(knobs
        .iter()
        .all(|k| k["path"].as_str().unwrap().starts_with("alerts.")));
    assert!(knobs.iter().any(|k| k["path"] == "alerts.cooldown_secs"));
    // health against a now-dead admin => exit code 1, still JSON on stdout
    let (rc, out, _) = cli(&[
        "health",
        "--config",
        cfg_path.to_str().unwrap(),
        "--url",
        &addr,
        "--json",
    ]);
    assert_eq!(rc, 1, "unreachable probe must not exit 0");
    assert!(out.contains("\"unreachable\""), "{out}");
    // offline-only health (no admin configured at all) => lane checks, exit 0
    let plain = dir.join("plain.json");
    let cfgv: Value = serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let mut pv = cfgv.clone();
    // empty string (not a missing key): the default addr is 127.0.0.1:9678
    pv["metrics_addr"] = json!("");
    std::fs::write(&plain, pv.to_string()).unwrap();
    let (rc, out, err) = cli(&["health", "--config", plain.to_str().unwrap(), "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["status"], "healthy", "{v}");

    let _ = std::fs::remove_dir_all(&dir);
    std::env::remove_var(TOK_ENV);
}

#[tokio::test(flavor = "current_thread")]
async fn bounded_listings_under_big_history() {
    // 5,000 history lines: the API window is capped at 512 parsed lines and
    // every page stays <= 200 items; nothing allocates from the file size.
    let dir = std::env::temp_dir().join(format!("harvest-opsapi-bound-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let hist = dir.join("runs.jsonl");
    {
        let mut f = std::fs::File::create(&hist).unwrap();
        for i in 0..5000 {
            writeln!(
                f,
                "{{\"run\":\"big\",\"ts\":{i},\"status\":\"ok\",\"duration_ms\":1,\"summary\":{{\"written\":{}}}}}",
                i % 10
            )
            .unwrap();
        }
    }
    let st = std::sync::Arc::new(harvest::admin::AdminState::new(
        std::sync::Arc::new(harvest::metrics::Metrics::default()),
        std::sync::Arc::new(harvest::proxy::ProxyPool::new(&[])),
        hist.clone(),
        None,
    ));
    let s2 = st.clone();
    let server = tokio::spawn(async move {
        let (client, server) = tokio::io::duplex(256 * 1024);
        let h = tokio::spawn(async move {
            let mut server = server;
            harvest::admin::handle(&mut server, &s2).await
        });
        let mut client = client;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        client
            .write_all(b"GET /runs?limit=10000 HTTP/1.1\r\nhost: x\r\n\r\n")
            .await
            .unwrap();
        let mut out = Vec::new();
        client.read_to_end(&mut out).await.unwrap();
        h.await.unwrap().unwrap();
        String::from_utf8_lossy(&out).into_owned()
    })
    .await
    .unwrap();
    let body = server.split("\r\n\r\n").nth(1).unwrap();
    let v: Value = serde_json::from_str(body).unwrap();
    assert_eq!(v["window"], 512, "parse window is capped at 512 lines");
    assert_eq!(v["page"]["total"], 512);
    assert_eq!(v["page"]["limit"], 200, "limit clamped, not honored raw");
    assert_eq!(v["page"]["items"].as_array().unwrap().len(), 200);
    assert!(
        !body.contains("harvest-opsapi-bound"),
        "no raw temp path: {body}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
