//! Final-release smoke: the whole pipeline on one temporary tree.
//!
//! Mock origin → `harvest::engine::run` (JSONL sink + strict-schema
//! quarantine + audit report) and → `harvest::runner::serve` (run history
//! JSONL) → `harvest::export` → `harvest::verify` → `harvest::artifact`
//! classification → `harvest::search` over the history file. Every layer is
//! used through its real public API against real files — the closest we get
//! to "run the shipped binary" inside an integration test (the CLI suite
//! already proves argv wiring; this one proves the parts compose).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// minimal mock origin (same hand-rolled HTTP trick as tests/e2e.rs)
// ---------------------------------------------------------------------------

fn respond(sock: &mut TcpStream) {
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = vec![0u8; 4096];
    let mut req = Vec::new();
    loop {
        let n = match sock.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        req.extend_from_slice(&buf[..n]);
        if req.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let line = String::from_utf8_lossy(&req)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let host = format!("127.0.0.1:{}", sock.local_addr().unwrap().port());
    let body = match path.as_str() {
        "/robots.txt" => "User-agent: *\nAllow: /\nDisallow: /private/\n".to_string(),
        "/" => format!(
            "<html><body><h1>Shop index</h1>\
             <a href=\"http://{host}/p/1\">1</a><a href=\"http://{host}/p/2\">2</a>\
             <a href=\"http://{host}/p/3\">3</a>\
             <a href=\"http://{host}/private/secret\">secret</a></body></html>"
        ),
        p if p.starts_with("/p/") => {
            format!("<html><body><h1>Page {p}</h1><p class=\"price\">$19.99</p></body></html>")
        }
        p => panic!("mock origin has no route {p}"),
    };
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(resp.as_bytes());
    let _ = sock.flush();
}

fn spawn_server() -> String {
    let lst = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = lst.local_addr().unwrap();
    std::thread::spawn(move || {
        while let Ok((sock, _)) = lst.accept() {
            std::thread::spawn(move || {
                let mut sock = sock;
                respond(&mut sock);
            });
        }
    });
    format!("http://{addr}")
}

// ---------------------------------------------------------------------------
// temp dir
// ---------------------------------------------------------------------------

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!(
            "harvest-final-{}-{}-{}",
            std::process::id(),
            tag,
            n
        ));
        std::fs::create_dir_all(&p).unwrap();
        Dir(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_pipeline_release_smoke() {
    let base = spawn_server();
    let dir = Dir::new("smoke");

    // -- crawl a two-target config ------------------------------------------
    let mut cfg: harvest::config::Config = {
        let v = json!({
            "name": "release-smoke",
            "user_agent": "HarvestTest/0.0 (final-release-smoke)",
            "respect_robots": true,
            "concurrency": 2,
            "max_retries": 2,
            "retry_base_ms": 5,
            "max_depth": 2,
            "max_pages": 30,
            "default_policy": { "requests_per_sec": 5000.0, "burst": 100, "min_interval_ms": 0 },
            "metrics_addr": "127.0.0.1:0",
            "audit_path": dir.0.join("audit.json"),
            "schedule": { "every_secs": 3600, "runs": 1,
                          "history_path": dir.0.join("unused-history.jsonl") },
            "targets": [
                {
                    "id": "lenient",
                    "start_url": format!("{base}/"),
                    "max_depth": 2,
                    "follow_links": true,
                    "extract": { "title": { "css": "h1" }, "price": { "css": ".price" } }
                },
                {
                    "id": "strict",
                    "start_url": format!("{base}/p/4"),
                    "max_depth": 1,
                    "follow_links": false,
                    "extract": { "title": { "css": "h1" }, "price": { "css": ".price" } },
                    "schema": {
                        "enforce": "strict",
                        // title is prose, not an integer: the mismatch must
                        // divert the record (currency coercion of "$19.99"
                        // would only ever count a drift)
                        "fields": { "title": "int", "price": "number" },
                        "quarantine_path": dir.0.join("quarantine.jsonl")
                    }
                }
            ],
            // sink is a top-level config key: one output for the whole run
            "output": { "kind": "jsonl", "path": dir.0.join("records.jsonl") }
        });
        serde_json::from_value(v).unwrap()
    };
    cfg.validate()
        .expect("the smoke config must pass real validation");

    let summary = harvest::engine::run(cfg.clone())
        .await
        .expect("engine run must succeed against the mock origin");
    assert_eq!(summary.written, 4, "index + three products: {summary:?}");
    assert_eq!(
        summary.quarantined, 1,
        "the strict target diverted its violating record: {summary:?}"
    );
    assert!(
        summary.robots_denied >= 1,
        "/private/ is Disallowed and linked from the index: {summary:?}"
    );

    // sink lines: the pinned record envelope {url, fields{...}, ...}
    let lines: Vec<Value> =
        BufReader::new(std::fs::File::open(dir.0.join("records.jsonl")).unwrap())
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect();
    assert_eq!(lines.len(), 4);
    assert!(lines.iter().all(|l| l["url"].is_string()));
    assert!(lines.iter().all(|l| l["fields"]["title"].is_string()));

    // quarantine rows: the strict mode's receipt
    let q: Vec<Value> =
        BufReader::new(std::fs::File::open(dir.0.join("quarantine.jsonl")).unwrap())
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect();
    assert_eq!(q.len(), 1, "exactly the violating record was diverted");

    // audit report: a single pretty-JSON document, read via the typed reader
    let report = harvest::repository::audit_report(&cfg)
        .unwrap()
        .expect("audit_path was configured and the run finished");
    assert!(report.is_object());

    // -- run history (written by the serve loop, the real producer) ---------
    {
        let hist_cfg: harvest::config::Config = serde_json::from_value(json!({
            "name": "release-smoke-hist",
            "user_agent": "HarvestTest/0.0 (final-release-smoke)",
            "respect_robots": true,
            "concurrency": 1,
            "max_retries": 0,
            "metrics_addr": "",
            "default_policy": { "requests_per_sec": 10000.0, "burst": 50, "min_interval_ms": 0 },
            "schedule": { "every_secs": 0, "runs": 1,
                          "history_path": dir.0.join("history.jsonl") },
            "targets": [{ "id": "h", "start_url": format!("{base}/p/3"),
                           "follow_links": false, "max_depth": 1,
                           "extract": { "title": { "css": "h1" } } }],
            "output": { "kind": "null" }
        }))
        .unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        harvest::runner::serve(hist_cfg, stop)
            .await
            .expect("bounded serve loop (runs:1) finishes on its own");

        let hist = harvest::repository::history_records(&cfg, 8).unwrap();
        // cfg.schedule.history_path points at a file no run wrote: the
        // absent-file rule is empty, not broken
        assert!(hist.records.is_empty(), "absent history file reads empty");

        let mut cfg_h = cfg.clone();
        cfg_h.schedule.history_path = dir.0.join("history.jsonl");
        let hist = harvest::repository::history_records(&cfg_h, 8).unwrap();
        assert_eq!(
            hist.records.len(),
            1,
            "serve recorded one run row (scanned={})",
            hist.stats.lines_scanned
        );

        // search sees the same row through the public query layer
        let sq = harvest::search::SearchQuery {
            resource: harvest::api_models::SearchResource::Runs,
            limit: 10,
            ..Default::default()
        };
        let page = harvest::search::search(&cfg_h, &sq).unwrap();
        assert_eq!(page.rows.len(), 1);
        assert_eq!(page.page.returned, 1);
        assert_eq!(page.page.next_offset, None, "window is at its end");
        assert!(
            page.rows[0]["ts"].as_u64().is_some(),
            "run rows carry the ts sort key the search layer promises"
        );
    }

    // -- export / verify / artifact ------------------------------------------
    let opts = harvest::export::ExportOpts {
        inputs: vec![dir.0.join("records.jsonl")],
        out_dir: dir.0.join("delivery"),
        split_target: true,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: Vec::new(),
        dataset: None,
        pdf_sample_rows: 12,
    };
    let exp = harvest::export::run(&opts).unwrap();
    assert_eq!(exp.manifest.records, 4);
    let out = Path::new(&exp.out_dir);

    let v = harvest::export::verify(out, None).unwrap();
    assert!(v.ok, "fresh delivery verifies: {:?}", v.error);
    assert_eq!(v.signature, "unsigned");
    let art = harvest::artifact::load(out).unwrap();
    assert_eq!(art.identity.records, 4);
    assert_eq!(
        harvest::artifact::classify(&v),
        harvest::artifact::ValidationState::VerifiedUnsigned,
        "verified payload, signature not checked (no key)"
    );

    // tamper: one flipped byte in the payload must change the verdict
    // (file names come from the manifest — split exports are per-target)
    let payload = out.join(&art.files[0].name);
    let mut bytes = std::fs::read(&payload).unwrap();
    let flip = bytes
        .iter_mut()
        .find(|b| **b == b'a')
        .expect("payload has an 'a'");
    *flip = b'b';
    std::fs::write(&payload, &bytes).unwrap();
    let v2 = harvest::export::verify(out, None).unwrap();
    assert!(!v2.ok, "flipped byte must fail verification");
    assert_eq!(
        harvest::artifact::classify(&v2),
        harvest::artifact::ValidationState::Tampered
    );
}

#[test]
fn audit_report_absent_is_none_not_error() {
    let dir = Dir::new("audit-none");
    let mut cfg = harvest::config::Config {
        audit_path: Some(dir.0.join("never-written.json")),
        ..harvest::config::Config::default()
    };
    cfg.name = "audit-none".into();
    let r = harvest::repository::audit_report(&cfg).unwrap();
    assert!(
        r.is_none(),
        "missing report file reads as absent, not broken"
    );
}
