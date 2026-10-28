//! Remote crawl execution through the authenticated control plane, against
//! deterministic local targets only (no public websites, no DNS, no external
//! network).
//!
//! Each test stands up its own mock origin + `harvest serve`, submits a
//! `POST /control/crawl`, and asserts over the real job lifecycle:
//! authentication, validation, target selection, limit enforcement, run
//! creation, artifact registration, audit and cancellation.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOK: &str = "remote-crawl-test-token-0123456789";

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
    sock.set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let auth = token
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: localhost\r\n{auth}content-length: {}\r\n\r\n{body}",
        body.len()
    );
    sock.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    let _ = sock.read_to_end(&mut raw);
    let text = String::from_utf8_lossy(&raw).into_owned();
    let split = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(text.len());
    let head = &text[..split];
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
    (status, text[split..].to_string(), headers)
}

fn http_bytes(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, Vec<u8>, Vec<(String, String)>) {
    let (status, text, headers) = http(addr, method, path, token, body);
    (status, text.into_bytes(), headers)
}

/// A local origin with three pages. `delay_ms` makes every response slow,
/// which is what the cancellation test needs (a crawl that is still running
/// when the cancel request arrives).
fn mock_origin(hits: Arc<AtomicUsize>, delay_ms: u64) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming().take(5000) {
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
            if delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(delay_ms));
            }
            let body = if path == "/robots.txt" {
                "User-agent: *\nAllow: /\n".to_string()
            } else if path == "/" || path.starts_with("/index") {
                "<html><body><h1>Catalogue</h1>\
                 <a href=\"/p/1\">one</a><a href=\"/p/2\">two</a><a href=\"/p/3\">three</a>\
                 </body></html>"
                    .to_string()
            } else if let Some(id) = path.strip_prefix("/p/") {
                format!("<html><body><h1>Item {id}</h1><p class=price>${id}.50</p></body></html>")
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

struct Fixture {
    dir: PathBuf,
    addr: String,
    proc: Option<ServeProc>,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("harvest-rc-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Fixture {
            dir,
            addr: String::new(),
            proc: None,
        }
    }

    /// `delay_ms` > 0 makes the origin slow (cancellation test).
    fn start(&mut self, base: &str, max_pages: usize, delay_ms: u64) {
        let addr = free_addr();
        let cfg = json!({
            "name": "remote-crawl-it",
            "respect_robots": true,
            "concurrency": 2,
            "max_pages": max_pages,
            "request_timeout_secs": 5,
            "metrics_addr": addr,
            "admin_token": TOK,
            "schedule": {"every_secs": 3600, "runs": 0,
                         "history_path": self.dir.join("out/runs.jsonl")},
            "output": {"kind": "jsonl", "path": self.dir.join("out/records.jsonl")},
            "targets": [
                {"id": "catalogue", "start_url": format!("{base}/index.html"),
                 "follow_links": true, "max_depth": 1, "max_pages": max_pages,
                 "extract": {"title": {"css": "h1"}, "price": {"css": "p.price"}}},
                {"id": "spare", "start_url": format!("{base}/p/9"),
                 "follow_links": false, "max_depth": 0, "max_pages": 2,
                 "extract": {"title": {"css": "h1"}}}
            ]
        });
        let cfg_path = self.dir.join("cfg.json");
        std::fs::write(&cfg_path, cfg.to_string()).unwrap();
        self.addr = addr.clone();
        self.proc = Some(ServeProc::start(&cfg_path));
        let t0 = Instant::now();
        loop {
            assert!(self.alive(), "serve died during startup");
            assert!(t0.elapsed() < Duration::from_secs(20), "admin never bound");
            if TcpStream::connect(&addr).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = delay_ms;
    }

    fn alive(&mut self) -> bool {
        self.proc.as_mut().map(|p| p.alive()).unwrap_or(false)
    }

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
                t0.elapsed() < Duration::from_secs(40),
                "the startup run never wrote records"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

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
                t0.elapsed() < Duration::from_secs(90),
                "job {id} never finished (state {state})"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn submit(&self, body: &Value) -> (u16, Value) {
        let (st, out, _) = http(
            &self.addr,
            "POST",
            "/control/crawl",
            Some(TOK),
            &body.to_string(),
        );
        (st, serde_json::from_str(&out).unwrap_or(Value::Null))
    }

    fn history_lines(&self) -> usize {
        std::fs::read_to_string(self.dir.join("out/runs.jsonl"))
            .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
            .unwrap_or(0)
    }

    fn audit(&self) -> Vec<Value> {
        let (st, body, _) = http(&self.addr, "GET", "/audit?limit=100", Some(TOK), "");
        assert_eq!(st, 200, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        v["items"].as_array().cloned().unwrap_or_default()
    }
}

// --------------------------------------------------------------------- tests

#[test]
fn crawl_submission_is_fail_closed_on_authentication() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits, 0);
    let mut fx = Fixture::new("auth");
    fx.start(&base, 20, 0);
    let _ = fx.wait_for_records();

    let body = json!({"op":"crawl","targets":["catalogue"]}).to_string();
    let (st, out, headers) = http(&fx.addr, "POST", "/control/crawl", None, &body);
    assert_eq!(st, 401, "{out}");
    assert!(headers
        .iter()
        .any(|(k, v)| k == "www-authenticate" && v == "Bearer"));
    let (st2, out2, _) = http(
        &fx.addr,
        "POST",
        "/control/crawl",
        Some("wrong-token"),
        &body,
    );
    assert_eq!(st2, 401, "{out2}");
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["error"],
        serde_json::from_str::<Value>(&out2).unwrap()["error"],
        "the 401 body is content-constant"
    );
    // and no job was created by the denied requests
    let (_, jobs, _) = http(&fx.addr, "GET", "/control/jobs", Some(TOK), "");
    let v: Value = serde_json::from_str(&jobs).unwrap();
    assert_eq!(v["total"], 0);
}

#[test]
fn target_selection_and_command_validation() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits, 0);
    let mut fx = Fixture::new("select");
    fx.start(&base, 20, 0);
    let _ = fx.wait_for_records();

    // the config declares two targets, so "all of them" is ambiguous
    let (st, out) = fx.submit(&json!({"op":"crawl"}));
    assert_eq!(st, 400, "{out}");
    assert!(out["error"]["message"]
        .as_str()
        .unwrap()
        .contains("declares 2 targets"));

    let (st, out) = fx.submit(&json!({"op":"crawl","targets":["ghost"]}));
    assert_eq!(st, 400, "{out}");
    assert!(out["error"]["message"]
        .as_str()
        .unwrap()
        .contains("unknown target"));

    let (st, out) = fx.submit(&json!({"op":"crawl","targets":["catalogue","nope"]}));
    assert_eq!(st, 400, "{out}");

    let (st, out) = fx.submit(&json!({"op":"crawl","targets":["catalogue","../etc"]}));
    assert_eq!(st, 400, "ids are charset-checked: {out}");

    // the export route refuses a crawl command outright
    let (st, out, _) = http(
        &fx.addr,
        "POST",
        "/control/export",
        Some(TOK),
        &json!({"op":"crawl","targets":["catalogue"]}).to_string(),
    );
    assert_eq!(st, 400, "{out}");

    // and no job exists after all of that
    let (_, jobs, _) = http(&fx.addr, "GET", "/control/jobs", Some(TOK), "");
    let v: Value = serde_json::from_str(&jobs).unwrap();
    assert_eq!(v["total"], 0);
}

#[test]
fn a_remote_crawl_creates_a_run_counters_and_an_artifact() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits.clone(), 0);
    let mut fx = Fixture::new("run");
    fx.start(&base, 20, 0);
    let _ = fx.wait_for_records();
    let history_before = fx.history_lines();

    let (st, v) = fx.submit(&json!({"op":"crawl","targets":["catalogue"]}));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["operation"], "crawl");
    assert_eq!(v["resource"], "catalogue");
    assert!(v["limits"]["max_pages"].as_u64().unwrap() >= 1);

    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "completed", "{done}");
    assert!(
        done["counters"]["pages_fetched"].as_u64().unwrap() >= 1,
        "the crawl actually fetched pages: {done}"
    );
    assert_eq!(done["counters"]["failures"].as_u64().unwrap(), 0, "{done}");
    let summary = &done["result"];
    assert!(summary["duration_ms"].as_u64().unwrap() > 0 || summary.is_null());

    // the run lands in the same history ledger the scheduler writes
    assert!(
        fx.history_lines() > history_before,
        "a control-plane crawl is a run"
    );

    // ... and its output is registered as a downloadable artifact
    let arts = done["artifacts"].as_array().unwrap();
    assert!(!arts.is_empty(), "{done}");
    let run_out = arts
        .iter()
        .find(|a| a["role"] == "run_output")
        .expect("the run output is registered");
    assert_eq!(run_out["state"], "ready");
    assert_eq!(run_out["name"], "records.jsonl");

    // download it: the bytes are the real sink envelope
    let (st, bytes, headers) = http_bytes(
        &fx.addr,
        "GET",
        &format!("/artifacts/{}", run_out["id"].as_str().unwrap()),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);
    let text = String::from_utf8_lossy(&bytes);
    let first: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
    assert!(first["url"].as_str().unwrap().starts_with("http"));
    assert!(first["fields"]["title"].is_string());
    let ctype = headers
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    assert!(ctype.contains("ndjson"), "{ctype}");

    // audit: submit, start and finish, with the target as the resource
    let rows = fx.audit();
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(actions.contains(&"control.crawl.submit"), "{actions:?}");
    assert!(actions.contains(&"control.crawl.start"), "{actions:?}");
    assert!(actions.contains(&"control.crawl.finish"), "{actions:?}");
    let submit_row = rows
        .iter()
        .find(|r| r["action"] == "control.crawl.submit")
        .unwrap();
    assert_eq!(submit_row["resource"], "catalogue");
    assert_eq!(submit_row["result"], "accepted");
    assert!(fx.alive());
}

#[test]
fn crawl_limits_are_enforced_by_the_backend() {
    let hits = Arc::new(AtomicUsize::new(0));
    let base = mock_origin(hits, 0);
    let mut fx = Fixture::new("limits");
    fx.start(&base, 20, 0);
    let _ = fx.wait_for_records();

    // ask for one page: the engine must not wander past it
    let (st, v) = fx.submit(&json!({
        "op":"crawl","targets":["catalogue"],
        "limits": {"max_pages": 1, "max_depth": 0, "timeout_secs": 60}
    }));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "completed", "{done}");
    assert!(
        done["counters"]["pages_fetched"].as_u64().unwrap() <= 1,
        "max_pages is a ceiling, not a hint: {}",
        done["counters"]
    );
    assert_eq!(done["limits"]["max_pages"], 1);

    // a budget above the ceiling is refused before any work starts
    let (st, out) =
        fx.submit(&json!({"op":"crawl","targets":["catalogue"],"limits":{"max_pages":10_000_000}}));
    assert_eq!(st, 400, "{out}");
    assert!(out["error"]["message"]
        .as_str()
        .unwrap()
        .contains("ceiling"));
}

#[test]
fn a_running_crawl_can_be_cancelled_and_says_so() {
    let hits = Arc::new(AtomicUsize::new(0));
    // every response takes 250 ms, so a 200-page crawl is still running when
    // the cancel request arrives
    let base = mock_origin(hits, 250);
    let mut fx = Fixture::new("cancel");
    fx.start(&base, 200, 250);
    let _ = fx.wait_for_records();

    let (st, v) = fx.submit(&json!({
        "op":"crawl","targets":["catalogue"],
        "limits": {"max_pages": 200, "max_depth": 2, "timeout_secs": 120}
    }));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();

    // wait until the job is genuinely running, then cancel it
    let t0 = Instant::now();
    loop {
        let (_, body, _) = http(
            &fx.addr,
            "GET",
            &format!("/control/jobs/{id}"),
            Some(TOK),
            "",
        );
        let snap: Value = serde_json::from_str(&body).unwrap();
        if snap["state"] == "running" {
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(20),
            "job never reached running: {snap}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    let (st, body, _) = http(
        &fx.addr,
        "POST",
        &format!("/control/jobs/{id}/cancel"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{body}");
    let cancelled: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(cancelled["cancel_requested"], true, "{cancelled}");
    assert!(
        cancelled["state"] == "cancelled" || cancelled["state"] == "running",
        "a running job either stops now or at its next checkpoint: {cancelled}"
    );

    let final_state = fx.wait_job(&id);
    assert_eq!(final_state["state"], "cancelled", "{final_state}");
    assert!(
        final_state["error"]
            .as_str()
            .unwrap_or("")
            .contains("cancel"),
        "the reason is recorded: {final_state}"
    );
    // cancelling again is a conflict — the job is already terminal
    let (st, _, _) = http(
        &fx.addr,
        "POST",
        &format!("/control/jobs/{id}/cancel"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 409);

    let rows = fx.audit();
    assert!(
        rows.iter().any(|r| r["action"] == "control.cancel"),
        "cancellation is audited: {rows:?}"
    );
    assert!(fx.alive(), "cancelling a crawl never kills the server");

    // and the server still serves the next command
    let (st, v) = fx.submit(&json!({"op":"crawl","targets":["spare"],"limits":{"max_pages":1}}));
    assert_eq!(st, 202, "{v}");
    let done = fx.wait_job(v["id"].as_str().unwrap());
    assert_eq!(done["state"], "completed", "{done}");
}
