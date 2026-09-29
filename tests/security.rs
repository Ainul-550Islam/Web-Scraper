//! Security & hardening integration tests: response-size caps, bounded
//! retries, redirect limits, config rejection — all against a canned,
//! deliberately hostile HTTP server, so they are deterministic and need no
//! external services.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use harvest::Config;

/// Canned server: returns per-path responses; `/huge` lies about length via
/// a giant content-length header, `/liar` streams chunked past the cap,
/// `/flaky` is a permanent 500, `/rN` is a redirect chain.
type Hits = Arc<Mutex<HashMap<String, usize>>>;

fn spawn() -> (String, Hits, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits: Arc<Mutex<HashMap<String, usize>>> = Default::default();
    let total = Arc::new(AtomicUsize::new(0));
    let (h, t) = (hits.clone(), total.clone());
    std::thread::spawn(move || {
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
                continue; // readiness probe: no response, no counting
            }
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            *h.lock().unwrap().entry(path.clone()).or_default() += 1;
            t.fetch_add(1, Ordering::SeqCst);
            let _ = match path.as_str() {
                "/robots.txt" => {
                    let b = "User-agent: *\n";
                    s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}", b.len()).as_bytes())
                }
                "/huge" => {
                    // Declares 20 MB up front: the engine must refuse at the
                    // header, before reading any body.
                    let _ = s.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: 20000000\r\nconnection: close\r\n\r\n");
                    Ok(())
                }
                "/liar" => {
                    // Chunked with NO content-length: keeps streaming past
                    // the cap; the engine must abort mid-stream.
                    let hdr = b"HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n";
                    if s.write_all(hdr).is_err() { continue; }
                    let chunk = vec![b'A'; 16 * 1024];
                    let mut ok = true;
                    for _ in 0..64 {
                        ok &= s.write_all(format!("{:x}\r\n", chunk.len()).as_bytes()).is_ok()
                            && s.write_all(&chunk).is_ok()
                            && s.write_all(b"\r\n").is_ok();
                        if !ok { break; }
                    }
                    let _ = s.write_all(b"0\r\n\r\n");
                    Ok(())
                }
                "/flaky" => s.write_all(b"HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"),
                p if p.starts_with("/r") => {
                    let n: u32 = p[2..].parse().unwrap_or(99);
                    if n >= 8 {
                        let b = "<html><body><h1>found</h1></body></html>";
                        s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{b}", b.len()).as_bytes())
                    } else {
                        s.write_all(
                            format!(
                                "HTTP/1.1 302 Found\r\nlocation: /r{}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                                n + 1
                            )
                            .as_bytes(),
                        )
                    }
                }
                _ => s
                    .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"),
            };
            let _ = s.flush();
        }
    });
    (format!("http://{addr}"), hits, total)
}

fn cfg(base: &str, dir: &std::path::Path, extra: serde_json::Value) -> Config {
    let mut raw = serde_json::json!({
        "name": "sec",
        "user_agent": "harvest-sectest/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 1,
        "max_retries": 0,
        "retry_base_ms": 5,
        "max_pages": 5,
        "metrics_addr": "",
        "request_timeout_secs": 10,
        "max_response_bytes": 32768,
        "default_policy": { "requests_per_sec": 5000.0, "burst": 50, "min_interval_ms": 0 },
        "targets": [{
            "id": "t",
            "start_url": format!("{base}/huge"),
            "max_depth": 0,
            "follow_links": false,
            "max_pages": 5,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "jsonl", "path": dir.join("out.jsonl").to_str().unwrap() }
    });
    if let (Some(o), Some(e)) = (raw.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    let mut cfg: Config = serde_json::from_value(raw).unwrap();
    if let Some(u) = extra.get("start_path").and_then(|x| x.as_str()) {
        cfg.targets[0].start_url = format!("{base}{u}");
    }
    cfg.validate().expect("sec test config must be valid");
    cfg
}

#[tokio::test]
async fn response_size_caps_reject_without_retry_or_oom() {
    let (base, hits, _) = spawn();
    let dir = std::env::temp_dir().join(format!("harvest-sec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    // (a) declared-too-large: refused at the header.
    let s = harvest::engine::run(cfg(&base, &dir, serde_json::json!({"start_path": "/huge"})))
        .await
        .unwrap();
    assert_eq!(s.written, 0, "oversized page must produce no record");
    assert_eq!(
        s.per_target["t"].failed, 1,
        "oversize must fail the URL: {s:?}"
    );
    assert_eq!(
        s.retries, 0,
        "a size-cap refusal is PERMANENT: retrying cannot shrink the body"
    );
    assert_eq!(
        hits.lock().unwrap().get("/huge").copied(),
        Some(1),
        "must not hammer after refusing"
    );

    // (b) lied-about-length / unbounded stream: aborted mid-stream.
    hits.lock().unwrap().clear();
    let s = harvest::engine::run(cfg(&base, &dir, serde_json::json!({"start_path": "/liar"})))
        .await
        .unwrap();
    assert_eq!(s.written, 0);
    assert_eq!(s.per_target["t"].failed, 1);
    assert_eq!(s.retries, 0, "mid-stream cap must also be permanent");
    assert_eq!(hits.lock().unwrap().get("/liar").copied(), Some(1));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn retries_are_bounded_to_max_retries() {
    let (base, hits, _) = spawn();
    let dir = std::env::temp_dir().join(format!("harvest-sec500-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let c = cfg(
        &base,
        &dir,
        serde_json::json!({ "start_path": "/flaky", "max_retries": 2, "retry_base_ms": 1 }),
    );
    let s = harvest::engine::run(c).await.unwrap();
    // initial + exactly max_retries retries — and no more, ever.
    assert_eq!(
        hits.lock().unwrap().get("/flaky").copied(),
        Some(3),
        "1 attempt + 2 retries"
    );
    assert_eq!(s.retries, 2);
    assert_eq!(s.per_target["t"].failed, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn redirect_chains_are_capped() {
    let (base, hits, _) = spawn();
    let dir = std::env::temp_dir().join(format!("harvest-secred-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let s = harvest::engine::run(cfg(&base, &dir, serde_json::json!({"start_path": "/r1"})))
        .await
        .unwrap();
    let h = hits.lock().unwrap();
    // reqwest follows at most 5 redirects: /r1../r6 are seen, /r7+/r8 never.
    assert_eq!(h.get("/r1").copied(), Some(1));
    assert_eq!(h.get("/r6").copied(), Some(1));
    assert_eq!(h.get("/r7"), None, "redirect cap not enforced");
    assert_eq!(h.get("/r8"), None);
    assert_eq!(
        s.per_target["t"].failed, 1,
        "too-many-redirects surfaces as failure: {s:?}"
    );
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn insecure_configs_are_rejected_by_the_cli_with_actionable_errors() {
    let bin = env!("CARGO_BIN_EXE_harvest");
    let dir = std::env::temp_dir().join(format!("harvest-seccli-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let write = |name: &str, body: String| {
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        p
    };

    // open admin on a routable bind => hard error naming the remedy
    let p = write(
        "open-admin.json",
        r#"{"targets":[{"id":"t","start_url":"http://127.0.0.1:1/","extract":{"x":{"css":"h1"}}}],
            "metrics_addr":"0.0.0.0:9678","output":{"kind":"jsonl","path":"out/x.jsonl"}}"#
            .to_string(),
    );
    let out = std::process::Command::new(bin)
        .args(["validate", "--config"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(!out.status.success(), "must reject open admin API");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("admin_token"), "actionable remedy in: {err}");

    // traversal in output path => rejected
    let p = write(
        "traversal.json",
        r#"{"targets":[{"id":"t","start_url":"http://127.0.0.1:1/","extract":{"x":{"css":"h1"}}}],
            "output":{"kind":"jsonl","path":"out/../../../../etc/evil.jsonl"}}"#
            .to_string(),
    );
    let out = std::process::Command::new(bin)
        .args(["validate", "--config"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains(".."),
        "path guard message"
    );

    // ${ENV} secret reference that is unset => loud failure naming the var
    let p = write(
        "envref.json",
        r#"{"targets":[{"id":"t","start_url":"http://127.0.0.1:1/","extract":{"x":{"css":"h1"}}}],
            "admin_token":"${HARVEST_DEFINITELY_NOT_EXPORTED}",
            "output":{"kind":"jsonl","path":"out/x.jsonl"}}"#
            .to_string(),
    );
    let out = std::process::Command::new(bin)
        .args(["validate", "--config"])
        .arg(&p)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("HARVEST_DEFINITELY_NOT_EXPORTED"),
        "missing env var must be named, not silently literal"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn attest_key_lifecycle_is_owner_only_and_verify_fails_closed() {
    let bin = env!("CARGO_BIN_EXE_harvest");
    let dir = std::env::temp_dir().join(format!("harvest-secatt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("k.hex");
    let art = dir.join("evidence.json");
    std::fs::write(&art, b"{\"requests\":1}\n").unwrap();

    let out = std::process::Command::new(bin)
        .args(["attest", "--gen-key"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            std::fs::metadata(&key).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    // refuse to clobber a live key (silently rotating keys would orphan receipts)
    let out = std::process::Command::new(bin)
        .args(["attest", "--gen-key"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(!out.status.success(), "overwrite of existing key must fail");

    // verify without any .sig must fail closed — not "skip = OK"
    let out = std::process::Command::new(bin)
        .args(["attest", "--verify"])
        .arg(&art)
        .args(["--key"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "missing signature file must not pass verification"
    );

    // engine-signed artifact path: sign manually through the same code the
    // engine uses, then the CLI must accept it.
    let k = harvest::sign::Key::load(&key).unwrap();
    let sig = k.sign_file(&art).unwrap();
    let out = std::process::Command::new(bin)
        .args(["attest", "--verify"])
        .arg(&art)
        .args(["--key"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(out.status.success(), "signed artifact must verify: {out:?}");
    assert!(sig.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
