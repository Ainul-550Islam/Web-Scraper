//! Artifact download and verification safety, driven over HTTP.
//!
//! The interesting properties are the ones that fail quietly: an id that
//! escapes the delivery directory, a download whose bytes drifted from its
//! manifest, a response that leaks where the file lives. Each is a test here,
//! and each is answered by the registry, not by the route.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const TOK: &str = "artifact-download-test-token-0123";

fn free_addr() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let a = l.local_addr().unwrap().to_string();
    drop(l);
    a
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

fn http(
    addr: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: &str,
) -> (u16, String, Vec<(String, String)>) {
    let (st, bytes, headers) = http_bytes(addr, method, path, token, body);
    (st, String::from_utf8_lossy(&bytes).into_owned(), headers)
}

fn header<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
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
        let dir = std::env::temp_dir().join(format!("harvest-ad-{}-{}", std::process::id(), tag));
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
            "name": "artifact-download-it",
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

    fn run_export(&self, body: &Value) -> Value {
        let (st, out, _) = http(
            &self.addr,
            "POST",
            "/control/export",
            Some(TOK),
            &body.to_string(),
        );
        assert_eq!(st, 202, "{out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        let id = v["id"].as_str().unwrap().to_string();
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
            let snap: Value = serde_json::from_str(&body).unwrap();
            let state = snap["state"].as_str().unwrap_or("");
            if !matches!(state, "queued" | "running") {
                assert_eq!(state, "completed", "{snap}");
                return snap;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "export never finished: {snap}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn artifact_named(&self, job: &Value, name: &str) -> Value {
        job["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["name"] == name)
            .unwrap_or_else(|| panic!("{name} missing from {}", job["artifacts"]))
            .clone()
    }

    fn delivery_dir(&self, job_id: &str) -> PathBuf {
        self.dir.join("out").join("deliveries").join(job_id)
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

// --------------------------------------------------------------------- tests

#[test]
fn downloads_are_authenticated_and_ids_never_escape_the_registry() {
    let base = mock_origin();
    let mut fx = Fixture::new("safety");
    fx.start(&base);
    let job = fx.run_export(&json!({"op":"export","formats":["csv"],"verify":true}));
    let art = fx.artifact_named(&job, "records.csv");
    let id = art["id"].as_str().unwrap();

    // no token, and a wrong token, are the same refusal
    let (st, body, headers) = http(&fx.addr, "GET", &format!("/artifacts/{id}"), None, "");
    assert_eq!(st, 401, "{body}");
    assert!(header(&headers, "www-authenticate") == Some("Bearer"));
    let (st, body2, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{id}"),
        Some("wrong-token"),
        "",
    );
    assert_eq!(st, 401, "{body2}");
    assert!(!body.contains(TOK) && !body2.contains(TOK));
    // the verify route is gated the same way
    let (st, _, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{id}/verify"),
        None,
        "",
    );
    assert_eq!(st, 401);

    // ids that try to walk out of the delivery directory
    for bad in [
        "/artifacts/../../etc/passwd",
        "/artifacts/..%2f..%2fetc%2fpasswd",
        "/artifacts/%2e%2e%2f%2e%2e%2fetc%2fpasswd",
        "/artifacts/....//....//etc/passwd",
        "/artifacts/art-1/../../../etc/passwd",
        "/artifacts/./../../etc/passwd",
        // the config file sits next to the delivery root: reaching it would be
        // the same bug, one hop shorter
        "/artifacts/../cfg.json",
        "/artifacts/art-1/../../cfg.json",
    ] {
        let (st, body, _) = http(&fx.addr, "GET", bad, Some(TOK), "");
        assert_eq!(st, 404, "{bad} => {body}");
        assert!(!body.contains("root:"), "{bad} leaked /etc/passwd");
        assert!(!body.contains("admin_token"), "{bad} leaked the config");
        assert!(!body.contains(TOK), "{bad} leaked the token");
    }

    // and on the verify route, the refusals are identical
    for bad in [
        "/artifacts/../../etc/passwd/verify",
        "/artifacts/art-1/../../cfg.json/verify",
        "/artifacts/art-999999/verify",
    ] {
        let (st, body, _) = http(&fx.addr, "GET", bad, Some(TOK), "");
        assert_eq!(st, 404, "{bad} => {body}");
    }
    assert!(fx.alive());
}

#[test]
fn a_ready_artifact_downloads_with_headers_that_match_its_bytes() {
    let base = mock_origin();
    let mut fx = Fixture::new("headers");
    fx.start(&base);
    let job = fx.run_export(&json!({"op":"export","formats":["csv"],"verify":true}));
    let art = fx.artifact_named(&job, "records.csv");
    let id = art["id"].as_str().unwrap();

    let (st, bytes, headers) =
        http_bytes(&fx.addr, "GET", &format!("/artifacts/{id}"), Some(TOK), "");
    assert_eq!(st, 200);
    assert_eq!(
        header(&headers, "content-type"),
        Some("text/csv; charset=utf-8")
    );
    assert_eq!(
        header(&headers, "content-length"),
        Some(bytes.len().to_string()).as_deref()
    );
    assert_eq!(header(&headers, "x-artifact-id"), Some(id));
    assert_eq!(header(&headers, "x-artifact-state"), Some("ready"));
    assert_eq!(
        header(&headers, "x-artifact-sha256"),
        Some(art["sha256"].as_str().unwrap()),
        "the header is the registry's hash"
    );
    assert_eq!(
        header(&headers, "x-artifact-sha256"),
        Some(sha256(&bytes)).as_deref(),
        "and it is the hash of the bytes we actually received"
    );
    let cd = header(&headers, "content-disposition")
        .unwrap_or_default()
        .to_string();
    assert!(cd.starts_with("attachment;"), "{cd}");
    assert!(
        cd.contains(&format!(
            "filename=\"{name}\"",
            name = art["name"].as_str().unwrap()
        )),
        "{cd}"
    );
    assert!(!cd.contains('/'), "the filename is bare: {cd}");
    assert!(!cd.contains(".."), "{cd}");
    assert_eq!(
        bytes.len() as u64,
        art["bytes"].as_u64().unwrap(),
        "the registry's byte count is the file's"
    );
}

#[test]
fn content_types_follow_the_registry_for_every_format() {
    let base = mock_origin();
    let mut fx = Fixture::new("types");
    fx.start(&base);
    let job = fx.run_export(&json!({
        "op":"export","formats":["csv","jsonl","xlsx","pdf","parquet","xml","zip"],
        "dataset":"dl","verify":true
    }));

    let expected = [
        ("records.csv", "text/csv; charset=utf-8"),
        ("records.jsonl", "application/x-ndjson"),
        ("manifest.json", "application/json"),
        ("checksums.txt", "text/plain; charset=utf-8"),
        (
            "dataset-dl.xlsx",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        ("dataset-dl.pdf", "application/pdf"),
        ("dataset-dl.parquet", "application/vnd.apache.parquet"),
        ("dataset-dl.xml", "application/xml"),
        ("dataset-dl.zip", "application/zip"),
    ];
    for (name, ctype) in expected {
        let art = fx.artifact_named(&job, name);
        let (st, bytes, headers) = http_bytes(
            &fx.addr,
            "GET",
            &format!("/artifacts/{}", art["id"].as_str().unwrap()),
            Some(TOK),
            "",
        );
        assert_eq!(st, 200, "{name}");
        assert_eq!(header(&headers, "content-type"), Some(ctype), "{name}");
        assert_eq!(bytes.len() as u64, art["bytes"].as_u64().unwrap(), "{name}");
    }
}

#[test]
fn metadata_shows_everything_except_where_the_file_lives() {
    let base = mock_origin();
    let mut fx = Fixture::new("metadata");
    fx.start(&base);
    let job = fx.run_export(&json!({"op":"export","formats":["csv"],"verify":true}));
    let art = fx.artifact_named(&job, "records.csv");
    let id = art["id"].as_str().unwrap();

    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/control/artifacts/{id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{body}");
    let meta: Value = serde_json::from_str(&body).unwrap();
    let root = fx.dir.to_string_lossy().to_string();
    assert!(!body.contains(&root), "{body}");
    assert!(!body.contains("deliveries"), "{body}");
    assert!(!body.contains(TOK), "{body}");
    assert!(meta["artifact"].get("path").is_none(), "{meta}");
    // what it does show, so a client can decide whether to fetch it
    let a = &meta["artifact"];
    for key in [
        "id",
        "job",
        "name",
        "role",
        "bytes",
        "sha256",
        "content_type",
        "state",
        "downloadable",
        "created_ts",
    ] {
        assert!(!a[key].is_null(), "{key} is reported: {a}");
    }
    assert_eq!(a["verification"]["state"], "ok", "{a}");
}

#[test]
fn a_failed_verification_takes_the_download_away() {
    let base = mock_origin();
    let mut fx = Fixture::new("failed");
    fx.start(&base);
    let job = fx.run_export(&json!({"op":"export","formats":["csv","jsonl"],"verify":true}));
    let job_id = job["id"].as_str().unwrap().to_string();
    let csv = fx.artifact_named(&job, "records.csv");
    let csv_id = csv["id"].as_str().unwrap().to_string();

    // before: downloadable
    let (st, _, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{csv_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);

    // tamper behind the API's back, then re-verify through the API
    let path = fx.delivery_dir(&job_id).join("records.csv");
    let mut contents = std::fs::read(&path).unwrap();
    let last = contents.len() - 1;
    contents[last] = contents[last].wrapping_add(1);
    std::fs::write(&path, &contents).unwrap();

    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{csv_id}/verify"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200, "{body}");
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["state"], "failed", "{v}");

    // every artifact of that delivery stops being downloadable, together
    for (_name, a) in [
        ("csv", fx.artifact_named(&job, "records.csv")),
        ("jsonl", fx.artifact_named(&job, "records.jsonl")),
        ("manifest", fx.artifact_named(&job, "manifest.json")),
    ] {
        let id = a["id"].as_str().unwrap();
        let (st, body, _) = http(&fx.addr, "GET", &format!("/artifacts/{id}"), Some(TOK), "");
        assert_eq!(st, 409, "{_name}: {body}");
        assert!(body.contains("verification"), "{body}");
        assert!(
            !body.contains(&fx.dir.to_string_lossy().to_string()),
            "{body}"
        );
    }

    // the metadata route agrees, so a client can grey the button out first
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
    assert!(fx.alive());
}

#[test]
fn verification_is_refused_for_artifacts_that_are_not_deliveries() {
    let base = mock_origin();
    let mut fx = Fixture::new("not-delivery");
    fx.start(&base);

    // a crawl registers the run output, which is a real file but not a
    // manifest-backed delivery
    let (st, out, _) = http(
        &fx.addr,
        "POST",
        "/control/crawl",
        Some(TOK),
        &json!({"op":"crawl","targets":["catalogue"],"limits":{"max_pages":1}}).to_string(),
    );
    assert_eq!(st, 202, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let snap = {
        let t0 = Instant::now();
        loop {
            let (_, body, _) = http(
                &fx.addr,
                "GET",
                &format!("/control/jobs/{id}"),
                Some(TOK),
                "",
            );
            let s: Value = serde_json::from_str(&body).unwrap();
            if !matches!(s["state"].as_str(), Some("queued") | Some("running")) {
                break s;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "crawl never finished"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert_eq!(snap["state"], "completed", "{snap}");
    let run_out = snap["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["role"] == "run_output")
        .expect("the run output is registered")
        .clone();
    let art_id = run_out["id"].as_str().unwrap();

    // it downloads — it is a real, registered file ...
    let (st, bytes, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{art_id}"),
        Some(TOK),
        "",
    );
    assert_eq!(st, 200);
    assert_eq!(bytes.len() as u64, run_out["bytes"].as_u64().unwrap());

    // ... but it has no manifest, so "verify" says so instead of guessing
    let (st, body, _) = http(
        &fx.addr,
        "GET",
        &format!("/artifacts/{art_id}/verify"),
        Some(TOK),
        "",
    );
    assert!(
        st == 404 || st == 409,
        "an artifact with no delivery is not verifiable: {st} {body}"
    );
    assert!(body.contains("\"error\""), "{body}");
}

#[test]
fn the_download_cap_is_the_job_cap_a_client_can_see() {
    let base = mock_origin();
    let mut fx = Fixture::new("cap");
    fx.start(&base);

    // the registry refuses an artifact over the cap, and the export job
    // applies the same number before a file is written, so an oversized
    // artifact can never reach the download path at all
    let (st, out, _) = http(
        &fx.addr,
        "POST",
        "/control/export",
        Some(TOK),
        &json!({"op":"export","formats":["csv"],"limits":{"max_export_bytes":8192}}).to_string(),
    );
    assert_eq!(st, 202, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    assert_eq!(v["limits"]["max_export_bytes"], 8192, "{v}");

    let snap = {
        let t0 = Instant::now();
        loop {
            let (_, body, _) = http(
                &fx.addr,
                "GET",
                &format!("/control/jobs/{id}"),
                Some(TOK),
                "",
            );
            let s: Value = serde_json::from_str(&body).unwrap();
            if !matches!(s["state"].as_str(), Some("queued") | Some("running")) {
                break s;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "export never finished"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    // this delivery is well under the cap, so it succeeds, and the cap it was
    // measured against is the one reported back
    assert_eq!(snap["state"], "completed", "{snap}");
    assert_eq!(snap["limits"]["max_export_bytes"], 8192, "{snap}");
    for a in snap["artifacts"].as_array().unwrap() {
        assert!(
            a["bytes"].as_u64().unwrap() <= 8192,
            "no artifact exceeds the cap it was produced under: {a}"
        );
    }

    // ask for a cap the delivery cannot fit under and the job fails instead
    // of writing a truncated file: an oversized artifact never exists, so it
    // can never reach the download path
    let (st, out, _) = http(
        &fx.addr,
        "POST",
        "/control/export",
        Some(TOK),
        &json!({"op":"export","formats":["csv"],"limits":{"max_export_bytes":16}}).to_string(),
    );
    assert_eq!(st, 202, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let snap = {
        let t0 = Instant::now();
        loop {
            let (_, body, _) = http(
                &fx.addr,
                "GET",
                &format!("/control/jobs/{id}"),
                Some(TOK),
                "",
            );
            let s: Value = serde_json::from_str(&body).unwrap();
            if !matches!(s["state"].as_str(), Some("queued") | Some("running")) {
                break s;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "export never finished"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
    };
    assert_eq!(snap["state"], "failed", "{snap}");
    assert!(
        snap["error"]
            .as_str()
            .unwrap()
            .contains("over the 16 byte cap"),
        "{}",
        snap["error"]
    );
    assert!(
        snap["artifacts"].as_array().unwrap().is_empty(),
        "nothing was registered: {snap}"
    );
}
