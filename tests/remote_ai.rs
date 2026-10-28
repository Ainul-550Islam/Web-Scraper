//! Remote AI-assisted execution, end to end through the authenticated control
//! plane, with **no network and no provider credentials**.
//!
//! The provider table this build registers is local and deterministic, so
//! every assertion here is reproducible: the job reads the run output the
//! configuration declares (or operator-supplied fields), the existing AI
//! client does the round, and the job reports counts, bands and the reason
//! when the round could not help.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOK: &str = "remote-ai-test-token-0123456789abc";

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
    sock.set_read_timeout(Some(Duration::from_secs(20)))
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

fn mock_origin() -> String {
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
            let body = if path == "/robots.txt" {
                "User-agent: *\nAllow: /\n".to_string()
            } else if path == "/" || path.starts_with("/index") {
                let mut links = String::new();
                for i in 1..4 {
                    links.push_str(&format!("<a href=\"/p/{i}\">p{i}</a>"));
                }
                format!("<html><body><h1>Catalogue</h1>{links}</body></html>")
            } else if let Some(id) = path.strip_prefix("/p/") {
                format!(
                    "<html><body><h1>Item {id} — a rather long title that \
                     could easily be clipped by a downstream consumer if it \
                     were any longer than this sentence, which it is not</h1>\
                     <p class=price>${id}.50</p></body></html>"
                )
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

struct ServeProc(Child);

impl ServeProc {
    fn start(cfg: &Path) -> ServeProc {
        ServeProc(
            Command::new(env!("CARGO_BIN_EXE_harvest"))
                .args(["serve", "--config"])
                .arg(cfg)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn harvest serve"),
        )
    }
    fn alive(&mut self) -> bool {
        matches!(self.0.try_wait(), Ok(None))
    }
}

impl Drop for ServeProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Fixture {
    dir: PathBuf,
    addr: String,
    proc: Option<ServeProc>,
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("harvest-ai-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("out")).unwrap();
        Fixture {
            dir,
            addr: String::new(),
            proc: None,
        }
    }

    fn start(&mut self, base: &str) {
        let addr = free_addr();
        let cfg = json!({
            "name": "remote-ai-it",
            "respect_robots": true,
            "concurrency": 2,
            "max_pages": 20,
            "request_timeout_secs": 5,
            "metrics_addr": addr,
            "admin_token": TOK,
            "schedule": {"every_secs": 3600, "runs": 0,
                         "history_path": self.dir.join("out/runs.jsonl")},
            "output": {"kind": "jsonl", "path": self.dir.join("out/records.jsonl")},
            "targets": [{
                "id": "catalogue", "start_url": format!("{base}/index.html"),
                "follow_links": true, "max_depth": 1, "max_pages": 10,
                "extract": {"title": {"css": "h1"}, "price": {"css": "p.price"}}
            }]
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
        // wait for the startup run to finish (history line) so an AI job over
        // the run output has real records to read
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
    }

    fn alive(&mut self) -> bool {
        self.proc.as_mut().map(|p| p.alive()).unwrap_or(false)
    }

    fn submit_ai(&self, body: &Value) -> (u16, Value) {
        let (st, out, _) = http(
            &self.addr,
            "POST",
            "/control/ai",
            Some(TOK),
            &body.to_string(),
        );
        (st, serde_json::from_str(&out).unwrap_or(Value::Null))
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
                t0.elapsed() < Duration::from_secs(60),
                "job {id} never finished (state {state})"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn run_ai(&self, body: &Value) -> Value {
        let (st, v) = self.submit_ai(body);
        assert_eq!(st, 202, "{v}");
        let id = v["id"].as_str().unwrap().to_string();
        self.wait_job(&id)
    }

    fn output_sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes = std::fs::read(self.dir.join("out/records.jsonl")).unwrap();
        let mut h = Sha256::new();
        h.update(&bytes);
        format!("{:x}", h.finalize())
    }

    fn audit(&self) -> Vec<Value> {
        let (st, body, _) = http(&self.addr, "GET", "/audit?limit=100", Some(TOK), "");
        assert_eq!(st, 200, "{body}");
        let v: Value = serde_json::from_str(&body).unwrap();
        v["items"].as_array().cloned().unwrap_or_default()
    }
}

fn inline(name: &str, value: &str) -> Value {
    json!({"name": name, "value": value})
}

// --------------------------------------------------------------------- tests

#[test]
fn ai_commands_are_authenticated_and_validated() {
    let base = mock_origin();
    let mut fx = Fixture::new("validate");
    fx.start(&base);

    let body = json!({"op":"ai","task":"classify"}).to_string();
    let (st, out, _) = http(&fx.addr, "POST", "/control/ai", None, &body);
    assert_eq!(st, 401, "{out}");
    let (st, out, _) = http(&fx.addr, "POST", "/control/ai", Some("wrong"), &body);
    assert_eq!(st, 401, "{out}");

    let cases: Vec<(&str, Value, u16)> = vec![
        ("missing task", json!({"op":"ai"}), 400),
        ("unknown task", json!({"op":"ai","task":"summarise"}), 400),
        (
            "unknown provider",
            json!({"op":"ai","task":"classify","provider":"openai-large"}),
            400,
        ),
        (
            "hosted provider name",
            json!({"op":"ai","task":"classify","provider":"https://api.example.test/v1"}),
            400,
        ),
        (
            "confidence above 1",
            json!({"op":"ai","task":"classify","confidence":1.5}),
            400,
        ),
        (
            "confidence below 0",
            json!({"op":"ai","task":"classify","confidence":-0.1}),
            400,
        ),
        (
            "empty inline fields",
            json!({"op":"ai","task":"classify","source":{"kind":"inline","fields":[]}}),
            400,
        ),
        (
            "inline field name too long",
            json!({"op":"ai","task":"classify","source":{"kind":"inline",
                   "fields":[{"name":"n".repeat(65),"value":"v"}]}}),
            400,
        ),
        (
            "inline value too long",
            json!({"op":"ai","task":"classify","source":{"kind":"inline",
                   "fields":[{"name":"n","value":"v".repeat(5000)}]}}),
            400,
        ),
        (
            "external consent on the disabled provider",
            json!({"op":"ai","task":"classify","provider":"disabled","external_allowed":true}),
            400,
        ),
    ];
    for (label, body, want) in cases {
        let (st, out) = fx.submit_ai(&body);
        assert_eq!(st, want, "{label}: {body} => {out}");
        assert!(out["error"]["message"].as_str().unwrap().len() > 8, "{out}");
    }

    // the refusal names what *is* available, so the error is actionable
    let (_, out) = fx.submit_ai(&json!({"op":"ai","task":"classify","provider":"openai-large"}));
    let msg = out["error"]["message"].as_str().unwrap().to_string();
    assert!(msg.contains("deterministic-local"), "{msg}");
    assert!(msg.contains("disabled"), "{msg}");

    let (_, jobs, _) = http(&fx.addr, "GET", "/control/jobs", Some(TOK), "");
    let v: Value = serde_json::from_str(&jobs).unwrap();
    assert_eq!(v["total"], 0, "nothing was executed");
}

#[test]
fn the_local_provider_completes_over_the_run_output() {
    let base = mock_origin();
    let mut fx = Fixture::new("local");
    fx.start(&base);

    let done = fx.run_ai(&json!({
        "op":"ai","task":"classify","provider":"deterministic-local",
        "source": {"kind":"run_output","record_limit":10,"field_limit":8}
    }));
    assert_eq!(done["state"], "completed", "{done}");
    let r = &done["result"];
    assert_eq!(r["state"], "ok", "{r}");
    assert_eq!(r["task"], "classify");
    assert_eq!(r["provider"], "deterministic-local");
    assert_eq!(
        r["external"], false,
        "the local provider is not external: {r}"
    );
    assert_eq!(r["external_allowed"], false, "consent defaults to off: {r}");
    assert!(r["counts"]["fields"].as_u64().unwrap() > 0, "{r}");
    assert!(r["input_bytes"].as_u64().unwrap() > 0, "{r}");
    assert!(r["latency_ms"].as_u64().is_some(), "{r}");
    assert!(r.get("reason").unwrap().is_null(), "{r}");
    // no artifacts: an AI round produces metadata, not a delivery
    assert!(
        done["artifacts"].as_array().unwrap().is_empty(),
        "an AI job registers nothing to download: {done}"
    );
    // metadata only — never a credential, endpoint or prompt
    let text = done.to_string();
    for banned in ["api_key", "apikey", "authorization", "secret", "sk-"] {
        assert!(!text.to_ascii_lowercase().contains(banned), "{text}");
    }

    // the audit trail says what ran, with no payload
    let rows = fx.audit();
    let actions: Vec<&str> = rows.iter().filter_map(|r| r["action"].as_str()).collect();
    assert!(actions.contains(&"control.ai.submit"), "{actions:?}");
    assert!(actions.contains(&"control.ai.finish"), "{actions:?}");
    let submit = rows
        .iter()
        .find(|r| r["action"] == "control.ai.submit")
        .unwrap();
    assert_eq!(submit["result"], "accepted");
    assert!(submit["detail"].to_string().contains("deterministic-local"));
    assert!(
        !submit["detail"].to_string().contains("price"),
        "no field names"
    );
    assert!(fx.alive());
}

#[test]
fn the_disabled_provider_is_a_failed_job_with_a_reason() {
    let base = mock_origin();
    let mut fx = Fixture::new("disabled");
    fx.start(&base);

    let (st, v) = fx.submit_ai(&json!({"op":"ai","task":"normalize","provider":"disabled"}));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(
        done["error"].as_str().unwrap(),
        "ai assistance is disabled for this job",
        "{done}"
    );
    // the partial result is still inspectable — that is the point
    assert_eq!(done["result"]["state"], "disabled", "{done}");
    assert_eq!(done["result"]["provider"], "disabled");
    assert!(fx.alive(), "a refused AI round never kills the server");
}

#[test]
fn the_input_budget_is_enforced_before_the_round() {
    let base = mock_origin();
    let mut fx = Fixture::new("budget");
    fx.start(&base);

    let (st, v) = fx.submit_ai(&json!({
        "op":"ai","task":"extract","limits":{"max_ai_input_bytes":8},
        "source": {"kind":"inline","fields":[inline("title","a long enough value to overflow an eight byte budget")]}
    }));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["result"]["state"], "budget-exceeded", "{done}");
    assert!(
        done["error"].as_str().unwrap().contains("input cap"),
        "{}",
        done["error"]
    );
    assert!(
        done["result"]["suggestions"].as_array().unwrap().is_empty(),
        "no suggestions were produced: {done}"
    );
}

#[test]
fn confidence_thresholds_gate_what_is_reported() {
    let base = mock_origin();
    let mut fx = Fixture::new("thresholds");
    fx.start(&base);

    // fields whose deterministic value is present, so the merge rules have
    // something to decide about
    let source = json!({"kind":"inline","fields":[
        {"name":"title","value":"Item 7 — a rather long title that could easily be clipped by a downstream consumer if it were any longer than this sentence, which it is not","deterministic":"Item 7"},
        {"name":"price","value":"$7.50","deterministic":"$7.50"}
    ]});

    // nothing clears a 1.0 bar, so every suggestion is routed to review
    let strict = fx.run_ai(&json!({
        "op":"ai","task":"normalize","confidence":1.0,"source":source
    }));
    assert_eq!(strict["state"], "completed", "{strict}");
    let r = &strict["result"];
    assert_eq!(r["counts"]["accepted"], 0, "nothing is auto-accepted: {r}");
    assert_eq!(r["counts"]["fields"], 2, "{r}");
    let strict_actions: Vec<String> = r["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(strict_actions, vec!["reject", "reject"], "{r}");
    assert_eq!(
        r["counts"]["rejected"], 2,
        "a 1.0 bar rejects everything: {r}"
    );
    for s in r["suggestions"].as_array().unwrap() {
        assert!(s["confidence"].as_f64().unwrap() < 1.0, "{s}");
        assert_eq!(s["replaces_validated"], false, "{s}");
    }

    // a zero bar marks the same suggestions acceptable — and still applies
    // nothing over a validated value without `overwrite_validated`
    let loose = fx.run_ai(&json!({
        "op":"ai","task":"normalize","confidence":0.0,"source":source
    }));
    assert_eq!(loose["state"], "completed", "{loose}");
    let r2 = &loose["result"];
    let loose_actions: Vec<String> = r2["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["action"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(loose_actions, vec!["accept", "accept"], "{r2}");
    assert_eq!(r2["counts"]["accepted"], 0, "no silent overwrite: {r2}");
    assert_eq!(
        r2["counts"]["kept_validated"], 2,
        "the extracted value wins: {r2}"
    );

    // the two runs differ only in the threshold's verdict
    assert_ne!(strict_actions, loose_actions);
}

#[test]
fn an_ai_job_never_rewrites_the_extracted_data() {
    let base = mock_origin();
    let mut fx = Fixture::new("immutable");
    fx.start(&base);
    let before = fx.output_sha256();
    let size_before = std::fs::metadata(fx.dir.join("out/records.jsonl"))
        .unwrap()
        .len();

    // a successful round...
    let ok = fx.run_ai(&json!({
        "op":"ai","task":"extract","confidence":0.0,
        "source": {"kind":"run_output","record_limit":10,"field_limit":8}
    }));
    assert_eq!(ok["state"], "completed", "{ok}");
    // ...and a refused one
    let off = fx.run_ai(&json!({"op":"ai","task":"extract","provider":"disabled"}));
    assert_eq!(off["state"], "failed", "{off}");

    assert_eq!(fx.output_sha256(), before, "the run output is untouched");
    assert_eq!(
        std::fs::metadata(fx.dir.join("out/records.jsonl"))
            .unwrap()
            .len(),
        size_before
    );
}

#[test]
fn recorded_external_consent_does_not_summon_an_external_provider() {
    let base = mock_origin();
    let mut fx = Fixture::new("consent");
    fx.start(&base);

    // consent is honoured in the audit and cannot change the provider
    let done = fx.run_ai(&json!({
        "op":"ai","task":"label","provider":"deterministic-local",
        "external_allowed": true,
        "source": {"kind":"inline","fields":[inline("title","Item 9")]}
    }));
    assert_eq!(done["state"], "completed", "{done}");
    assert_eq!(done["result"]["external_allowed"], true, "{done}");
    assert_eq!(
        done["result"]["external"], false,
        "consent does not create an external provider: {done}"
    );

    let rows = fx.audit();
    let submit = rows
        .iter()
        .find(|r| r["action"] == "control.ai.submit")
        .unwrap();
    assert!(
        submit["detail"].to_string().contains("external_allowed"),
        "consent is recorded: {submit}"
    );
    assert!(fx.alive());
}
