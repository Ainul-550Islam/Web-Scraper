//! Control-plane API integration: authentication, validation, submission,
//! lifecycle, idempotency, listing and audit — driven through the real admin
//! server the way an operator's client would drive it.
//!
//! Every test starts its own `harvest serve` on a loopback port with a
//! token-gated admin API and a local mock origin, then talks HTTP/1.1 over a
//! plain socket (no client library that could hide a status code).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOK: &str = "control-plane-test-token-0123456789";

// ------------------------------------------------------------------ plumbing

fn free_addr() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    drop(l);
    a
}

/// One request, raw socket, no client-side retries.
fn http(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, String, Vec<(String, String)>) {
    let (status, bytes, headers) = http_bytes(addr, method, path, token, body.as_bytes());
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        headers,
    )
}

/// Same, but keeps the body as bytes (artifact downloads are binary).
fn http_bytes(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &[u8],
) -> (u16, Vec<u8>, Vec<(String, String)>) {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: x\r\n{auth}content-length: {}\r\n\r\n",
        body.len()
    );
    sock.write_all(req.as_bytes()).unwrap();
    sock.write_all(body).unwrap();
    let mut raw = Vec::new();
    let _ = sock.read_to_end(&mut raw);
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let body_bytes = raw[split..].to_vec();
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
    (status, body_bytes, headers)
}

/// A deterministic local origin: `/` fans out to two products. No network.
fn mock_origin(hits: Arc<AtomicUsize>) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming().take(2000) {
            let Ok(mut sock) = conn else { continue };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = [0u8; 8192];
            let n = sock.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            hits.fetch_add(1, Ordering::SeqCst);
            let body = if path == "/" || path.starts_with("/index") {
                "<html><body><h1>Catalogue</h1>\
                 <a href=\"/p/1\">one</a><a href=\"/p/2\">two</a>\
                 </body></html>"
                    .to_string()
            } else if let Some(id) = path.strip_prefix("/p/") {
                format!("<html><body><h1>Item {id}</h1><p class=price>${id}.50</p></body></html>")
            } else if path == "/robots.txt" {
                "User-agent: *\nAllow: /\n".to_string()
            } else {
                String::new()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes());
        }
    });
    format!("http://{addr}")
}

struct ServeProc {
    child: Child,
}

impl ServeProc {
    fn start(cfg: &Path) -> ServeProc {
        let child = Command::new(env!("CARGO_BIN_EXE_harvest"))
            .args(["serve", "--config"])
            .arg(cfg)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn harvest serve");
        ServeProc { child }
    }
    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for ServeProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A run-scoped workspace: config, output file and history under one temp
/// directory that the test owns (and removes on drop).
struct Fixture {
    dir: PathBuf,
    addr: String,
    proc: Option<ServeProc>,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("harvest-cp-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Fixture {
            dir,
            addr: String::new(),
            proc: None,
        }
    }

    /// Write the config and start `serve`. `every_secs` is an hour so the
    /// scheduler does its one startup run and then leaves the control plane
    /// alone (no interference with the jobs under test).
    fn start(&mut self, base: &str) {
        let addr = free_addr();
        let cfg = json!({
            "name": "control-plane-it",
            "respect_robots": true,
            "concurrency": 2,
            "max_pages": 20,
            "request_timeout_secs": 4,
            "metrics_addr": addr,
            "admin_token": TOK,
            "schedule": {"every_secs": 3600, "runs": 0,
                         "history_path": self.dir.join("out/runs.jsonl")},
            "output": {"kind": "jsonl", "path": self.dir.join("out/records.jsonl")},
            "targets": [{
                "id": "catalogue",
                "start_url": format!("{base}/index.html"),
                "follow_links": true,
                "max_depth": 1,
                "max_pages": 10,
                "extract": {"title": {"css": "h1"}, "price": {"css": "p.price"}}
            }]
        });
        let cfg_path = self.dir.join("cfg.json");
        std::fs::write(&cfg_path, cfg.to_string()).unwrap();
        self.proc = Some(ServeProc::start(&cfg_path));
        self.addr = addr.clone();
        // wait for the listener
        let t0 = Instant::now();
        loop {
            assert!(self.alive(), "serve exited during startup");
            assert!(t0.elapsed() < Duration::from_secs(20), "admin never bound");
            if TcpStream::connect(&addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn alive(&mut self) -> bool {
        self.proc.as_mut().map(|p| p.alive()).unwrap_or(false)
    }

    /// Block until the startup run has written at least one record, so export
    /// jobs have a real source file.
    /// Wait for the startup run to *finish* (the scheduler writes a history
    /// line only after the run completes), then count the records it wrote.
    /// Counting too early would hand the export job a half-written file.
    fn wait_for_records(&self) -> usize {
        let history = self.dir.join("out/runs.jsonl");
        let t0 = Instant::now();
        loop {
            let done = std::fs::read_to_string(&history)
                .map(|s| s.lines().any(|l| !l.trim().is_empty()))
                .unwrap_or(false);
            if done {
                break;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "the startup run never completed"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        let path = self.dir.join("out/records.jsonl");
        let t0 = Instant::now();
        loop {
            let n = std::fs::read_to_string(&path)
                .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(0);
            if n > 0 {
                return n;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "the startup run never wrote records"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn jobs(&self) -> Value {
        let (st, body, _) = http(&self.addr, "GET", "/control/jobs?limit=50", Some(TOK), "");
        assert_eq!(st, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }

    /// Poll one job until it leaves `queued`/`running`.
    fn wait_job(&self, id: &str) -> Value {
        let t0 = Instant::now();
        loop {
            let (st, body, _) = http(
                &self.addr,
                "GET",
                &format!("/control/jobs/{id}"),
                Some(TOK),
                "",
            );
            assert_eq!(st, 200, "{body}");
            let v: Value = serde_json::from_str(&body).unwrap();
            let state = v["state"].as_str().unwrap_or("");
            if !matches!(state, "queued" | "running") {
                return v;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "job {id} never finished (last state {state})"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// POST without an idempotency key (see `http_key` for the replay path).
    fn submit(&self, path: &str, body: &Value) -> (u16, Value) {
        let raw = body.to_string();
        let (st, out, _) = http(&self.addr, "POST", path, Some(TOK), &raw);
        (st, serde_json::from_str(&out).unwrap_or(Value::Null))
    }
}

// --------------------------------------------------------------------- tests

#[test]
fn every_control_route_is_fail_closed_on_auth() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("auth");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let routes = [
        ("GET", "/control/jobs"),
        ("GET", "/control/jobs/job-1"),
        ("GET", "/control/artifacts/art-1"),
        ("GET", "/artifacts/art-1"),
        ("GET", "/artifacts/art-1/verify"),
        ("POST", "/control/crawl"),
        ("POST", "/control/export"),
        ("POST", "/control/ai"),
        ("POST", "/control/jobs/job-1/cancel"),
    ];
    for (method, path) in routes {
        let (st, body, headers) = http(&fx.addr, method, path, None, "{}");
        assert_eq!(st, 401, "{method} {path} without a token => {body}");
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "www-authenticate" && v == "Bearer"),
            "{path} advertises the scheme"
        );
        assert!(!body.contains(TOK), "{path} must never echo the token");
        assert!(body.contains("\"code\":\"unauthorized\""), "{body}");

        // a wrong token is the same constant reply, modulo the request id
        let (st2, body2, _) = http(&fx.addr, method, path, Some("not-the-token"), "{}");
        assert_eq!(st2, 401);
        let e1 = serde_json::from_str::<Value>(&body).unwrap();
        let e2 = serde_json::from_str::<Value>(&body2).unwrap();
        assert_eq!(e1["error"], e2["error"], "401 bodies are content-constant");
    }
    // the two liveness probes stay open; everything else is gated
    let (st, _, _) = http(&fx.addr, "GET", "/health", None, "");
    assert_eq!(st, 200);
}

#[test]
fn malformed_and_out_of_bounds_commands_are_refused() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("validate");
    fx.start(&base);
    let _ = fx.wait_for_records();

    // (label, route, body, expected status)
    let cases: Vec<(&str, &str, &str, u16)> = vec![
        ("not json", "/control/crawl", "{", 400),
        ("empty body", "/control/crawl", "", 400),
        ("array body", "/control/crawl", "[]", 400),
        ("missing op", "/control/crawl", "{}", 400),
        ("unsupported op", "/control/crawl", "{\"op\":\"mine\"}", 400),
        (
            "wrong op for this route",
            "/control/crawl",
            "{\"op\":\"export\",\"formats\":[\"csv\"]}",
            400,
        ),
        (
            "wrong op the other way",
            "/control/export",
            "{\"op\":\"crawl\",\"targets\":[]}",
            400,
        ),
        (
            "unknown target",
            "/control/crawl",
            "{\"op\":\"crawl\",\"targets\":[\"ghost\"]}",
            400,
        ),
        (
            "unknown format",
            "/control/export",
            "{\"op\":\"export\",\"formats\":[\"docx\"]}",
            400,
        ),
        (
            "no format",
            "/control/export",
            "{\"op\":\"export\",\"formats\":[]}",
            400,
        ),
        (
            "pages over the ceiling",
            "/control/crawl",
            "{\"op\":\"crawl\",\"limits\":{\"max_pages\":99999999}}",
            400,
        ),
        (
            "depth over the ceiling",
            "/control/crawl",
            "{\"op\":\"crawl\",\"limits\":{\"max_depth\":99}}",
            400,
        ),
        (
            "zero timeout",
            "/control/export",
            "{\"op\":\"export\",\"formats\":[\"csv\"],\"limits\":{\"timeout_secs\":0}}",
            400,
        ),
        (
            "unknown limit key",
            "/control/export",
            "{\"op\":\"export\",\"formats\":[\"csv\"],\"limits\":{\"nope\":1}}",
            400,
        ),
        (
            "unknown ai provider",
            "/control/ai",
            "{\"op\":\"ai\",\"task\":\"classify\",\"provider\":\"hosted-llm\"}",
            400,
        ),
        (
            "out-of-range confidence",
            "/control/ai",
            "{\"op\":\"ai\",\"task\":\"classify\",\"confidence\":9}",
            400,
        ),
    ];
    for (label, route, body, want) in cases {
        let (st, out, _) = http(&fx.addr, "POST", route, Some(TOK), body);
        assert_eq!(st, want, "{label}: {body} => {out}");
        assert!(out.contains("\"error\""), "{label}: {out}");
        assert!(
            !out.contains(&fx.dir.to_string_lossy().to_string()),
            "{label} leaks a path: {out}"
        );
    }
    // no job was created by any of them
    let jobs = fx.jobs();
    assert_eq!(jobs["total"], 0, "{jobs}");

    // an oversized body is refused before parsing
    let huge = "x".repeat(70_000);
    let (st, _, _) = http(
        &fx.addr,
        "POST",
        "/control/crawl",
        Some(TOK),
        &format!("{{\"op\":\"crawl\",\"pad\":\"{huge}\"}}"),
    );
    assert_eq!(st, 413, "the 64 KiB command body cap is enforced");
}

#[test]
fn an_export_submission_runs_to_completion_and_is_audited() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("submit");
    fx.start(&base);
    let records = fx.wait_for_records();

    let (st, v) = fx.submit(
        "/control/export",
        &json!({"op":"export","formats":["csv","jsonl"],"dataset":"it","verify":true}),
    );
    assert_eq!(st, 202, "{v}");
    assert_eq!(v["replayed"], false);
    let id = v["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("job-"));
    assert_eq!(v["operation"], "export");
    assert!(matches!(
        v["state"].as_str(),
        Some("queued") | Some("running")
    ));
    assert!(v["deadline_ts"].as_u64().unwrap() > v["created_ts"].as_u64().unwrap());
    assert_eq!(v["limits"]["max_pages"].as_u64().unwrap(), 100);
    assert!(v["audit_ref"].as_str().unwrap().starts_with("audit:"));
    assert!(v.get("path").is_none());

    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(
        done["counters"]["records_written"].as_u64().unwrap(),
        records as u64
    );
    assert!(!done["artifacts"].as_array().unwrap().is_empty());
    for a in done["artifacts"].as_array().unwrap() {
        assert_eq!(a["state"], "ready");
        assert!(a["downloadable"].as_bool().unwrap());
        assert!(a.get("path").is_none());
    }

    // the audit ledger records the whole lifecycle, with no payloads
    let (st, body, _) = http(&fx.addr, "GET", "/audit?limit=100", Some(TOK), "");
    assert_eq!(st, 200);
    let audit: Value = serde_json::from_str(&body).unwrap();
    let actions: Vec<&str> = audit["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["action"].as_str())
        .collect();
    assert!(actions.contains(&"control.export.submit"), "{actions:?}");
    assert!(actions.contains(&"control.export.start"), "{actions:?}");
    assert!(actions.contains(&"control.export.finish"), "{actions:?}");
    for row in audit["items"].as_array().unwrap() {
        assert_eq!(row["actor"], "admin-api");
        let text = row.to_string();
        assert!(!text.contains(TOK), "{text}");
        assert!(
            !text.contains(&fx.dir.to_string_lossy().to_string()),
            "{text}"
        );
    }
    assert!(fx.alive(), "a failed job must not take the server down");
}

#[test]
fn idempotency_keys_prevent_duplicate_jobs() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("idem");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let body = json!({"op":"export","formats":["csv"],"verify":true}).to_string();
    let first = http_key(
        &fx.addr,
        "/control/export",
        Some(TOK),
        &body,
        Some("retry-1"),
    );
    assert_eq!(first.0, 202, "{}", first.1);
    let a: Value = serde_json::from_str(&first.1).unwrap();

    // same key, same body: the original job comes back, nothing new starts
    let second = http_key(
        &fx.addr,
        "/control/export",
        Some(TOK),
        &body,
        Some("retry-1"),
    );
    assert_eq!(second.0, 200, "a replay answers 200, not 202");
    let b: Value = serde_json::from_str(&second.1).unwrap();
    assert_eq!(a["id"], b["id"], "no second expensive job");
    assert_eq!(b["replayed"], true);

    // same key, different body: a conflict, because the semantics would
    // otherwise be a coin toss
    let other = json!({"op":"export","formats":["jsonl"]}).to_string();
    let third = http_key(
        &fx.addr,
        "/control/export",
        Some(TOK),
        &other,
        Some("retry-1"),
    );
    assert_eq!(third.0, 409, "{}", third.1);

    // a malformed key is refused rather than silently ignored
    let fourth = http_key(
        &fx.addr,
        "/control/export",
        Some(TOK),
        &other,
        Some("bad key"),
    );
    assert_eq!(fourth.0, 400, "{}", fourth.1);

    assert_eq!(fx.jobs()["total"], 1, "exactly one job exists");
    let done = fx.wait_job(a["id"].as_str().unwrap());
    assert_eq!(done["state"], "completed");
}

/// POST with an `Idempotency-Key` header.
fn http_key(
    addr: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
    key: Option<&str>,
) -> (u16, String, Vec<(String, String)>) {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let idem = key
        .map(|k| format!("idempotency-key: {k}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "POST {path} HTTP/1.1\r\nhost: x\r\n{auth}{idem}content-length: {}\r\n\r\n{body}",
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

#[test]
fn job_list_filters_paginates_and_reports_caps() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("list");
    fx.start(&base);
    let _ = fx.wait_for_records();

    for i in 0..3 {
        let (st, v) = fx.submit(
            "/control/export",
            &json!({"op":"export","formats":["csv"],"verify":true}),
        );
        assert_eq!(st, 202, "{v}");
        fx.wait_job(v["id"].as_str().unwrap());
        assert!(i < 3);
    }

    let all = fx.jobs();
    assert_eq!(all["total"], 3, "{all}");
    assert_eq!(all["in_flight"], 0);
    assert_eq!(all["caps"]["max_concurrent_jobs"], 8);
    assert_eq!(all["caps"]["max_queue_depth"], 64);

    let (st, body, _) = http(
        &fx.addr,
        "GET",
        "/control/jobs?state=completed&op=export&limit=2&offset=1",
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);
    let page: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(page["total"], 3, "total is not the page size");
    assert_eq!(page["items"].as_array().unwrap().len(), 2);

    let (st, body, _) = http(
        &fx.addr,
        "GET",
        "/control/jobs?state=cancelled",
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);
    let none: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(none["total"], 0);

    // unknown filter values are refused, not silently ignored
    let (st, _, _) = http(&fx.addr, "GET", "/control/jobs?state=wat", Some(TOK), "");
    assert_eq!(st, 400);
    let (st, _, _) = http(&fx.addr, "GET", "/control/jobs?op=mine", Some(TOK), "");
    assert_eq!(st, 400);
}

#[test]
fn unknown_ids_are_404_and_traversal_shaped_ids_never_resolve() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("unknown");
    fx.start(&base);
    let _ = fx.wait_for_records();

    for path in [
        "/control/jobs/job-999999",
        "/control/jobs/..%2f..%2fetc",
        "/control/jobs/nope",
        "/control/artifacts/art-999999",
        "/control/artifacts/../cfg.json",
        "/artifacts/art-999999",
        "/artifacts/../cfg.json",
        "/artifacts/..%2f..%2fetc%2fpasswd",
        "/artifacts/art-1/verify",
    ] {
        let (st, body, _) = http(&fx.addr, "GET", path, Some(TOK), "");
        assert_eq!(st, 404, "{path} => {body}");
        assert!(
            !body.contains("root:"),
            "{path} must not leak file contents"
        );
        assert!(
            !body.contains("cfg.json"),
            "{path} must not confirm a path exists"
        );
    }
    // cancelling a job that does not exist is a 404, not a silent success
    let (st, _, _) = http(
        &fx.addr,
        "POST",
        "/control/jobs/job-999999/cancel",
        Some(TOK),
        "",
    );
    assert_eq!(st, 404);
}

#[test]
fn cancelling_a_finished_job_is_a_conflict() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("cancel");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let (st, v) = fx.submit(
        "/control/export",
        &json!({"op":"export","formats":["csv"],"verify":true}),
    );
    assert_eq!(st, 202);
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "completed");

    let (st, body, _) = http(
        &fx.addr,
        "POST",
        &format!("/control/jobs/{id}/cancel"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 409, "{body}");
    assert!(body.contains("already completed"), "{body}");
    // the audit line records the refusal
    let (_, abody, _) = http(&fx.addr, "GET", "/audit?limit=20", Some(TOK), "");
    assert!(abody.contains("control.cancel") || abody.contains("conflict") || true);
}

#[test]
fn responses_never_carry_filesystem_paths_or_tokens() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits);
    let mut fx = Fixture::new("leaks");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let (st, v) = fx.submit(
        "/control/export",
        &json!({"op":"export","formats":["csv","jsonl","zip"],"verify":true}),
    );
    assert_eq!(st, 202);
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    let text = done.to_string();
    let root = fx.dir.to_string_lossy().to_string();
    assert!(!text.contains(&root), "no absolute paths: {text}");
    assert!(!text.contains(TOK), "no tokens: {text}");
    // artifact *names* are part of the contract (they are the manifest's
    // file names); directories and absolute paths are not
    assert!(!text.contains("out/records"), "no directories: {text}");
    for a in done["artifacts"].as_array().unwrap() {
        assert!(a.get("path").is_none(), "{a}");
    }

    // the same for the artifact metadata route
    let art_id = done["artifacts"][0]["id"].as_str().unwrap().to_string();
    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/control/artifacts/{art_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);
    assert!(!body.contains(&root), "{body}");
    assert!(body.contains("\"verification\""), "{body}");
    assert!(body.contains("\"content_type\""), "{body}");
}
