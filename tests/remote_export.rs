//! Remote export execution: every format the exporter implements, rendered
//! by the engine through the authenticated control plane, verified and
//! downloaded. No format logic is reimplemented here — the test reads the
//! bytes the engine wrote and checks the promises the manifest makes about
//! them.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOK: &str = "remote-export-test-token-0123456789";

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
    let (status, bytes, headers) = http_bytes(addr, method, path, token, body);
    (
        status,
        String::from_utf8_lossy(&bytes).into_owned(),
        headers,
    )
}

fn http_bytes(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, Vec<u8>, Vec<(String, String)>) {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
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
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| i + 4)
        .unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
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
    (status, raw[split..].to_vec(), headers)
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
                for i in 1..5 {
                    links.push_str(&format!("<a href=\"/p/{i}\">p{i}</a>"));
                }
                format!("<html><body><h1>Catalogue</h1>{links}</body></html>")
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
        let dir = std::env::temp_dir().join(format!("harvest-re-{}-{}", std::process::id(), tag));
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
            "name": "remote-export-it",
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

    fn submit_export(&self, body: &Value) -> (u16, Value) {
        let (st, out, _) = http(
            &self.addr,
            "POST",
            "/control/export",
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
                t0.elapsed() < Duration::from_secs(90),
                "job {id} never finished (state {state})"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn download(&self, id: &str) -> (u16, Vec<u8>, Vec<(String, String)>) {
        http_bytes(
            &self.addr,
            "GET",
            &format!("/artifacts/{id}"),
            Some(TOK),
            "",
        )
    }

    /// Where this job's delivery landed on disk. The API never exposes the
    /// path; the test owns the workspace, so it can compute it.
    fn delivery_dir(&self, job_id: &str) -> PathBuf {
        self.dir.join("out").join("deliveries").join(job_id)
    }

    /// One export job, waited for, asserted completed.
    fn run_export(&self, body: &Value) -> Value {
        let (st, v) = self.submit_export(body);
        assert_eq!(st, 202, "{v}");
        let id = v["id"].as_str().unwrap().to_string();
        let done = self.wait_job(&id);
        assert_eq!(done["state"], "completed", "{done}");
        done
    }
}

fn artifact_by_name(job: &Value, name: &str) -> Value {
    job["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing from {}", job["artifacts"]))
        .clone()
}

// --------------------------------------------------------------------- tests

#[test]
fn export_submission_validates_auth_and_formats() {
    let base = mock_origin();
    let mut fx = Fixture::new("validate");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let body = json!({"op":"export","formats":["csv"]}).to_string();
    let (st, out, _) = http(&fx.addr, "POST", "/control/export", None, &body);
    assert_eq!(st, 401, "{out}");
    let (st, out, _) = http(&fx.addr, "POST", "/control/export", Some("nope"), &body);
    assert_eq!(st, 401, "{out}");

    for bad in [
        json!({"op":"export","formats":[]}),
        json!({"op":"export","formats":["docx"]}),
        json!({"op":"export","formats":["csv","tar"]}),
        json!({"op":"export","formats":["zip"],"bundle":true}),
        json!({"op":"export","formats":["csv"],"limits":{"max_records":0}}),
        json!({"op":"export","formats":["csv"],"limits":{"max_export_bytes":0}}),
    ] {
        let (st, out) = fx.submit_export(&bad);
        assert_eq!(st, 400, "{bad} => {out}");
        assert!(
            out["error"]["message"].as_str().unwrap().len() > 10,
            "{out}"
        );
    }
    // the supported list is echoed in the refusal
    let (_, out) = fx.submit_export(&json!({"op":"export","formats":["docx"]}));
    let msg = out["error"]["message"].as_str().unwrap().to_string();
    for f in ["csv", "jsonl", "xlsx", "pdf", "parquet", "xml", "zip"] {
        assert!(msg.contains(f), "{msg}");
    }
    // format names are case- and space-insensitive, and normalise to the
    // canonical spelling in the artifacts they produce
    let (st, v) =
        fx.submit_export(&json!({"op":"export","formats":[" CSV ","JSONL"],"verify":true}));
    assert_eq!(st, 202, "{v}");
    let done = fx.wait_job(v["id"].as_str().unwrap());
    assert_eq!(done["state"], "completed", "{done}");
    let names: Vec<String> = done["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.iter().any(|n| n == "records.csv"), "{names:?}");
    assert!(names.iter().any(|n| n == "records.jsonl"), "{names:?}");
    assert!(fx.alive());
}

#[test]
fn all_seven_formats_render_and_download_as_real_files() {
    let base = mock_origin();
    let mut fx = Fixture::new("formats");
    fx.start(&base);
    let records = fx.wait_for_records();

    let done = fx.run_export(&json!({
        "op":"export","formats":["csv","jsonl","xlsx","pdf","parquet","xml","zip"],
        "dataset":"it-all","verify":true
    }));
    assert_eq!(
        done["counters"]["records_written"].as_u64().unwrap(),
        records as u64
    );

    let names: Vec<String> = done["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["name"].as_str().unwrap().to_string())
        .collect();
    for n in [
        "records.csv",
        "records.jsonl",
        "dataset-it-all.xlsx",
        "dataset-it-all.pdf",
        "dataset-it-all.parquet",
        "dataset-it-all.xml",
        "dataset-it-all.zip",
        "manifest.json",
        "checksums.txt",
    ] {
        assert!(names.iter().any(|x| x == n), "{n} missing from {names:?}");
    }

    // every payload downloads, and each is the real thing
    let csv = artifact_by_name(&done, "records.csv");
    let (st, bytes, headers) = fx.download(csv["id"].as_str().unwrap());
    assert_eq!(st, 200);
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.lines().count() >= records, "one row per record");
    assert!(text.contains("url"), "a header row: {text}");
    assert_eq!(
        headers.iter().find(|(k, _)| k == "content-type").unwrap().1,
        "text/csv; charset=utf-8"
    );

    let jsonl = artifact_by_name(&done, "records.jsonl");
    let (st, bytes, _) = fx.download(jsonl["id"].as_str().unwrap());
    assert_eq!(st, 200);
    for line in String::from_utf8_lossy(&bytes).lines() {
        let v: Value = serde_json::from_str(line).expect("each line is JSON");
        assert!(v["url"].is_string());
    }

    let xlsx = artifact_by_name(&done, "dataset-it-all.xlsx");
    let (st, bytes, _) = fx.download(xlsx["id"].as_str().unwrap());
    assert_eq!(st, 200);
    assert_eq!(&bytes[..2], b"PK", "an xlsx is a zip container");

    let pdf = artifact_by_name(&done, "dataset-it-all.pdf");
    let (st, bytes, _) = fx.download(pdf["id"].as_str().unwrap());
    assert_eq!(st, 200);
    assert_eq!(&bytes[..4], b"%PDF", "a pdf starts with %PDF");

    let parquet = artifact_by_name(&done, "dataset-it-all.parquet");
    let (st, bytes, _) = fx.download(parquet["id"].as_str().unwrap());
    assert_eq!(st, 200);
    assert_eq!(&bytes[..4], b"PAR1", "parquet magic");
    assert_eq!(&bytes[bytes.len() - 4..], b"PAR1", "parquet footer");

    let xml = artifact_by_name(&done, "dataset-it-all.xml");
    let (st, bytes, _) = fx.download(xml["id"].as_str().unwrap());
    assert_eq!(st, 200);
    let xml_text = String::from_utf8_lossy(&bytes).to_string();
    assert!(xml_text.trim_start().starts_with("<?xml") || xml_text.trim_start().starts_with('<'));
    assert!(xml_text.contains("<record"), "{xml_text}");
    assert!(xml_text.contains("type=\""), "typed fields: {xml_text}");

    let zip = artifact_by_name(&done, "dataset-it-all.zip");
    let (st, bytes, _) = fx.download(zip["id"].as_str().unwrap());
    assert_eq!(st, 200);
    assert_eq!(&bytes[..2], b"PK", "a zip starts with PK");
    assert_eq!(bytes.len() as u64, zip["bytes"].as_u64().unwrap());
}

#[test]
fn the_manifest_matches_the_payloads_and_verification_passes() {
    let base = mock_origin();
    let mut fx = Fixture::new("manifest");
    fx.start(&base);
    let records = fx.wait_for_records();

    let done = fx.run_export(&json!({
        "op":"export","formats":["csv","jsonl"],"verify":true
    }));
    assert_eq!(done["result"]["verification"]["state"], "ok", "{done}");

    // the manifest's hashes are the hashes of the bytes we can download
    let manifest = artifact_by_name(&done, "manifest.json");
    let (st, bytes, _) = fx.download(manifest["id"].as_str().unwrap());
    assert_eq!(st, 200);
    let man: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes)).unwrap();
    assert_eq!(man["records"], records as u64);
    let files = man["files"].as_array().unwrap();
    assert!(files.len() >= 2);
    for f in files {
        let name = f["name"].as_str().unwrap();
        let art = artifact_by_name(&done, name);
        let (st, bytes, _) = fx.download(art["id"].as_str().unwrap());
        assert_eq!(st, 200, "{name}");
        let digest = {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            h.update(&bytes);
            format!("{:x}", h.finalize())
        };
        assert_eq!(
            digest,
            f["sha256"].as_str().unwrap(),
            "{name} hash mismatch"
        );
        assert_eq!(
            bytes.len() as u64,
            f["bytes"].as_u64().unwrap(),
            "{name} size"
        );
    }

    // checksums.txt covers the same ground, in `sha256sum -c` shape
    let sums = artifact_by_name(&done, "checksums.txt");
    let (st, bytes, _) = fx.download(sums["id"].as_str().unwrap());
    assert_eq!(st, 200);
    let text = String::from_utf8_lossy(&bytes).to_string();
    assert!(text.lines().count() >= 2, "{text}");
    assert!(text.contains("records.csv"), "{text}");

    // and the verification endpoint agrees
    let art = artifact_by_name(&done, "records.csv");
    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{}/verify", art["id"].as_str().unwrap()),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["state"], "ok", "{v}");
    assert_eq!(v["detail"]["records_basis"], "jsonl-lines");
    assert_eq!(v["detail"]["records"], records as u64);
}

#[test]
fn tampering_is_detected_and_the_download_is_disabled() {
    let base = mock_origin();
    let mut fx = Fixture::new("tamper");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let done = fx.run_export(&json!({"op":"export","formats":["csv","jsonl"],"verify":true}));
    let id = done["id"].as_str().unwrap().to_string();
    let csv = artifact_by_name(&done, "records.csv");
    let csv_id = csv["id"].as_str().unwrap().to_string();

    // flip a byte in the delivered file, behind the API's back
    let path = fx.delivery_dir(&id).join("records.csv");
    let mut body = std::fs::read(&path).unwrap();
    let last = body.len() - 1;
    body[last] = body[last].wrapping_add(1);
    std::fs::write(&path, &body).unwrap();

    // `harvest verify` — the CLI authority — refuses it
    let out = Command::new(env!("CARGO_BIN_EXE_harvest"))
        .args(["verify", "--dir"])
        .arg(fx.delivery_dir(&id))
        .output()
        .expect("run harvest verify");
    assert!(
        !out.status.success(),
        "a tampered delivery must not verify: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // the control plane's re-verify reaches the same verdict
    let (st, vbody, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{csv_id}/verify"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{vbody}");
    let v: Value = serde_json::from_str(&vbody).unwrap();
    assert_eq!(v["state"], "failed", "{v}");

    // ... and the artifact stops being downloadable
    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/control/artifacts/{csv_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{body}");
    let meta: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(meta["artifact"]["state"], "failed", "{meta}");
    assert_eq!(meta["artifact"]["downloadable"], false, "{meta}");

    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{csv_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 409, "{body}");
    assert!(body.contains("verification"), "{body}");
    assert!(fx.alive());
}

#[test]
fn a_deleted_payload_is_detected_too() {
    let base = mock_origin();
    let mut fx = Fixture::new("deleted");
    fx.start(&base);
    let _ = fx.wait_for_records();

    let done = fx.run_export(&json!({"op":"export","formats":["jsonl"],"verify":true}));
    let id = done["id"].as_str().unwrap().to_string();
    let jsonl = artifact_by_name(&done, "records.jsonl");
    let jsonl_id = jsonl["id"].as_str().unwrap().to_string();

    std::fs::remove_file(fx.delivery_dir(&id).join("records.jsonl")).unwrap();

    let (st, vbody, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{jsonl_id}/verify"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{vbody}");
    let v: Value = serde_json::from_str(&vbody).unwrap();
    assert_eq!(v["state"], "failed", "{v}");
    let (st, _, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{jsonl_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 409, "a missing file is not downloadable");
}

#[test]
fn export_limits_are_enforced_by_the_backend() {
    let base = mock_origin();
    let mut fx = Fixture::new("limits");
    fx.start(&base);
    let records = fx.wait_for_records();
    assert!(records >= 2, "need at least two records for this test");

    // record cap: the export stops at exactly N records
    let done = fx.run_export(&json!({
        "op":"export","formats":["jsonl"],"max_records":2,"verify":true
    }));
    assert_eq!(done["result"]["records"], 2, "{done}");
    let jsonl = artifact_by_name(&done, "records.jsonl");
    let (st, bytes, _) = fx.download(jsonl["id"].as_str().unwrap());
    assert_eq!(st, 200);
    assert_eq!(String::from_utf8_lossy(&bytes).lines().count(), 2);

    // byte cap: an over-cap delivery is a failed job, not a truncated file
    let (st, v) = fx.submit_export(&json!({
        "op":"export","formats":["csv","jsonl"],"limits":{"max_export_bytes":16}
    }));
    assert_eq!(st, 202, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let done = fx.wait_job(&id);
    assert_eq!(done["state"], "failed", "{done}");
    assert!(
        done["error"]
            .as_str()
            .unwrap()
            .contains("over the 16 byte cap"),
        "{done}"
    );
    assert!(
        done["artifacts"].as_array().unwrap().is_empty(),
        "a failed export registers nothing: {done}"
    );

    // the failed job is audited like any other
    let (_, abody, _) = http(&fx.addr, "GET", "/audit?limit=50", Some(TOK), "");
    let audit: Value = serde_json::from_str(&abody).unwrap();
    assert!(
        audit["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["action"] == "control.export.finish" && r["result"] == "failed"),
        "{audit}"
    );
}
