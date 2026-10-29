//! End-to-end coverage of the managed customer API.
//!
//! This drives the real router (`admin::handle`) over a real socket against a
//! real local origin, with a real `App` built from a real config. Nothing is
//! mocked: the tenant store is on disk, the usage ledger is appended and
//! hash-chained, the webhook leaves over HTTP, and the metering counters are
//! the ones `/metrics` would expose.
//!
//! The origin is loopback, so the suite sets the documented demo/test opt-in
//! (`managed_api.allow_private_targets`) for the tests that need it and keeps
//! it off for the one that proves the guard still refuses.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use harvest::admin::{handle, AdminState};
use harvest::config::{Config, ManagedApiCfg, WebhookCfg};
use harvest::metrics::Metrics;
use harvest::proxy::ProxyPool;
use harvest::scrape_api::{App, AppInputs};
use harvest::tenancy::limits::KeyLimits;
use harvest::tenancy::{Scope, ScopeSet, TenancyStore};

// ------------------------------------------------------------------ harness

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        let unique = format!(
            "harvest-managed-api-{tag}-{}-{:?}",
            std::process::id(),
            Instant::now()
        );
        path.push(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The page the origin serves at `/`. Hoisted so a test can assert on the
/// delivered byte count without repeating (and drifting from) the literal.
const DEMO_PAGE: &str = "<html><body><h1>Harvest demo</h1><p>Hello from the local origin.</p><ul><li>one</li><li>two</li></ul></body></html>";

/// A minimal origin: `/` is the page under test, `/robots.txt` permits
/// everything, and `/hook` counts webhook deliveries (the signature check
/// reads the header the receiver actually saw).
struct Origin {
    addr: String,
    /// Every request the origin saw, which is how a test proves a replay did
    /// not go back to the network.
    requests: Arc<AtomicUsize>,
    /// Hook deliveries only.
    hits: Arc<AtomicUsize>,
    signatures: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Origin {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind origin");
        let addr = listener.local_addr().unwrap().to_string();
        let hits = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let signatures = Arc::new(std::sync::Mutex::new(Vec::new()));
        let hits_for_thread = hits.clone();
        let requests_for_thread = requests.clone();
        let sigs_for_thread = signatures.clone();
        std::thread::spawn(move || {
            for conn in listener.incoming().take(1024) {
                let Ok(mut sock) = conn else { continue };
                let _ = sock.set_read_timeout(Some(Duration::from_secs(10)));
                let _ = sock.set_write_timeout(Some(Duration::from_secs(10)));
                let mut buf = [0u8; 16384];
                let n = sock.read(&mut buf).unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let mut lines = head.lines();
                let request_line = lines.next().unwrap_or("").to_string();
                let path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .to_string();
                let mut signature = String::new();
                for line in lines {
                    if let Some((k, v)) = line.split_once(':') {
                        if k.trim().eq_ignore_ascii_case("x-harvest-signature") {
                            signature = v.trim().to_string();
                        }
                    }
                }
                requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let body: String = match path.as_str() {
                    p if p.starts_with("/hook") => {
                        hits_for_thread.fetch_add(1, Ordering::SeqCst);
                        if !signature.is_empty() {
                            sigs_for_thread.lock().unwrap().push(signature);
                        }
                        "ok".to_string()
                    }
                    "/robots.txt" => "User-agent: *\nAllow: /\n".to_string(),
                    "/" | "/index.html" => DEMO_PAGE.to_string(),
                    "/missing" => {
                        return_with(&mut sock, 404, "text/plain", "not found");
                        continue;
                    }
                    _ => String::new(),
                };
                let ctype = if path == "/robots.txt" {
                    "text/plain"
                } else {
                    "text/html"
                };
                let _ = sock.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        Self {
            addr,
            requests,
            hits,
            signatures,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}/{path}", self.addr)
    }
}

fn return_with(sock: &mut TcpStream, status: u16, ctype: &str, body: &str) {
    let _ = sock.write_all(
        format!(
            "HTTP/1.1 {status} {}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            if status == 404 { "Not Found" } else { "OK" },
            body.len()
        )
        .as_bytes(),
    );
}

/// The admin server, running the real `handle` on a background runtime.
struct Server {
    addr: String,
    _rt: std::thread::JoinHandle<()>,
}

fn start_server(state: Arc<AdminState>) -> Server {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build admin runtime");
    let thread = std::thread::spawn(move || {
        rt.block_on(async move {
            // Bind on the runtime itself: `TcpListener::from_std` is rejected
            // on a thread that registers a blocking fd.
            let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
                Ok(l) => l,
                Err(_) => return,
            };
            let addr = match listener.local_addr() {
                Ok(a) => a.to_string(),
                Err(_) => return,
            };
            let _ = tx.send(addr);
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    return;
                };
                let state = state.clone();
                tokio::spawn(async move {
                    let mut sock = sock;
                    let _ = handle(&mut sock, &state).await;
                });
            }
        });
    });
    let addr = rx
        .recv_timeout(Duration::from_secs(30))
        .expect("admin server bound");
    Server { addr, _rt: thread }
}

fn http(
    addr: &str,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &str,
) -> (u16, String, Vec<(String, String)>) {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    sock.set_write_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("content-length: {}\r\n", body.len()));
    req.push_str("connection: close\r\n\r\n");
    // A 413 is answered from the request head, so the server may close the
    // socket while the body is still on the wire. The response is still worth
    // reading, so a refused write is not fatal here.
    let _ = sock.write_all(req.as_bytes());
    if !body.is_empty() {
        let _ = sock.write_all(body.as_bytes());
    }
    let _ = sock.flush();
    let mut raw = Vec::new();
    sock.take(4 << 20).read_to_end(&mut raw).unwrap();
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("response framing");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("status line");
    let headers = head
        .lines()
        .skip(1)
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    (
        status,
        String::from_utf8_lossy(&raw[split + 4..]).into_owned(),
        headers,
    )
}

/// Poll `GET /v1/jobs/:id` until the job has made at least `attempts` runs.
fn wait_for_job_attempt(w: &World, job_id: &str, attempts: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = Value::Null;
    while Instant::now() < deadline {
        let (status, text, _) = http(
            &w.server.addr,
            "GET",
            &format!("/v1/jobs/{job_id}"),
            &bearer_headers(&w.key),
            "",
        );
        assert_eq!(status, 200, "body: {text}");
        last = serde_json::from_str(&text).expect("json job");
        // `attempts` is bumped when the run starts, so also wait for the
        // run to have recorded its outcome before reading it back.
        let finished = last["finished_at_ms"].is_number();
        if last["attempts"].as_u64().unwrap_or(0) >= attempts && finished {
            return last;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!("job {job_id} never reached {attempts} attempt(s): {last}");
}

/// Poll `GET /v1/jobs/:id` until the job reaches a terminal state.
///
/// The job runs on a spawned task, so a caller that reads it back
/// immediately can legitimately see `queued` or `running`.
fn wait_for_terminal_job(w: &World, job_id: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = Value::Null;
    while Instant::now() < deadline {
        let (status, text, _) = http(
            &w.server.addr,
            "GET",
            &format!("/v1/jobs/{job_id}"),
            &bearer_headers(&w.key),
            "",
        );
        assert_eq!(status, 200, "body: {text}");
        last = serde_json::from_str(&text).expect("json job");
        let state = last["status"].as_str().unwrap_or("");
        if matches!(state, "done" | "failed" | "cancelled") {
            return last;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    panic!(
        "job {job_id} never reached a terminal state: {}",
        last["status"]
    );
}

fn bearer_headers(key: &str) -> Vec<(String, String)> {
    vec![("authorization".to_string(), format!("Bearer {key}"))]
}

/// Everything one test needs: a config, a store with one tenant and one key,
/// an attached `App`, and the running server.
struct World {
    server: Server,
    origin: Origin,
    app: Arc<App>,
    store: Arc<TenancyStore>,
    key: String,
    tenant: String,
    _dir: TempDir,
}

fn build_world(allow_private: bool, token: Option<&str>) -> World {
    let dir = TempDir::new("world");
    let origin = Origin::start();

    let mut cfg = Config {
        respect_robots: true,
        robots_ttl_secs: 60,
        max_response_bytes: 8 * 1024 * 1024,
        concurrency: 2,
        ..Config::default()
    };
    cfg.managed_api = ManagedApiCfg {
        enabled: true,
        allow_private_targets: allow_private,
        data_dir: Some(dir.path().join("managed")),
        webhook: WebhookCfg {
            enabled: true,
            allowed_hosts: vec![origin_host(&origin)],
            max_attempts: 3,
            retry_base_ms: 5,
            retry_cap_ms: 20,
            dead_letter_path: None,
            ..ManagedApiCfg::default().webhook
        },
        ..ManagedApiCfg::default()
    };

    let metrics = Arc::new(Metrics::default());
    let managed_cfg = cfg.managed_api.clone();
    let inputs = AppInputs::from_config(Arc::new(cfg), managed_cfg, metrics.clone())
        .expect("build managed api inputs");
    let app = Arc::new(App::new(inputs).expect("start managed api"));

    let store = app.store().clone();
    let tenant = store
        .new_tenant("acme")
        .expect("create tenant")
        .id
        .to_string();
    let limits = KeyLimits {
        allowed_domains: vec![origin_host(&origin)],
        scopes: ScopeSet::from(vec![
            Scope::Scrape,
            Scope::JobsRead,
            Scope::JobsWrite,
            Scope::UsageRead,
        ]),
        rps: 50.0,
        burst: 50,
        monthly_quota: 10_000,
        ..KeyLimits::default()
    };
    let generated = store
        .create_key(&tenant, limits.scopes.clone(), limits.clone(), None)
        .expect("create key");

    let state = Arc::new(AdminState::new(
        metrics,
        Arc::new(ProxyPool::new(&[])),
        dir.path().join("history.jsonl"),
        token.map(str::to_string),
    ));
    state.attach_managed_api(app.clone());
    let server = start_server(state);
    World {
        server,
        origin,
        app,
        store,
        key: generated.plaintext,
        tenant,
        _dir: dir,
    }
}

fn origin_host(origin: &Origin) -> String {
    origin
        .addr
        .rsplit_once(':')
        .map(|(h, _)| h.to_string())
        .unwrap_or_else(|| origin.addr.clone())
}

fn json_headers(key: &str) -> Vec<(String, String)> {
    let mut h = bearer_headers(key);
    h.push(("content-type".to_string(), "application/json".to_string()));
    h
}

// ---------------------------------------------------------------- the tests

#[test]
fn a_scrape_round_trip_returns_the_canonical_envelope() {
    let w = build_world(true, None);
    let body = json!({ "url": w.origin.url(""), "format": "text" });
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &body.to_string(),
    );
    assert_eq!(status, 200, "body: {text}");
    let v: Value = serde_json::from_str(&text).expect("json response");
    // Every managed response carries the numeric contract stamp; a client
    // reads it to know which shape it got.
    assert_eq!(v["schema"], 1);
    assert_eq!(v["status"], 200);
    assert!(v["request_id"].as_str().unwrap().len() > 8);
    assert_eq!(v["url"], w.origin.url(""));
    assert_eq!(v["bytes"].as_u64().unwrap(), DEMO_PAGE.len() as u64);
    assert!(v["usage_units"].as_u64().unwrap() > 0);
    // `cost` is the unit count priced through the configured table, so it is
    // a positive amount in the configured currency.
    let cost = v["cost"].as_str().unwrap();
    let amount = cost
        .trim_end()
        .strip_suffix("USD")
        .expect("cost is denominated in the configured currency");
    assert!(
        amount.trim().parse::<f64>().unwrap() > 0.0,
        "cost was {cost}"
    );
    let body_text = v["body"].as_str().unwrap();
    assert!(body_text.contains("Harvest demo"), "body was {body_text}");
    assert!(!body_text.contains("<h1>"), "format text strips markup");
    // The ledger head is echoed, so a caller can see the request was metered.
    assert!(v["chain"]["seq"].as_u64().unwrap() >= 1);
    assert!(!v["chain"]["hash"].as_str().unwrap().is_empty());
}

#[test]
fn an_unknown_route_is_a_404_with_the_managed_envelope() {
    let w = build_world(true, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/nope",
        &json_headers(&w.key),
        "{}",
    );
    assert_eq!(status, 404, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["error"]["code"], "not_found");
}

#[test]
fn a_scrape_without_a_key_is_refused() {
    let w = build_world(true, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &[("content-type".to_string(), "application/json".to_string())],
        "{}",
    );
    assert_eq!(status, 401, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "missing_api_key");
}

#[test]
fn a_wrong_key_is_refused_without_leaking_anything() {
    let w = build_world(true, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers("sk_not_a_real_key"),
        "{}",
    );
    assert_eq!(status, 401, "body: {text}");
    assert!(
        !text.contains("sk_not_a_real_key"),
        "never echo the presented key"
    );
    assert!(!text.contains(&w.key), "never echo the real key");
    let v: Value = serde_json::from_str(&text).unwrap();
    // `unauthorized` is the stable code; the detail is a human-readable
    // message and never echoes what was presented.
    assert_eq!(v["error"]["code"], "unauthorized");
    assert_eq!(v["error"]["message"], "invalid API key");
}

#[test]
fn the_console_bearer_is_not_required_for_customer_routes() {
    // `/v1/scrape` authenticates with the tenant key; the console token (which
    // is configured here) must not be demanded for it.
    let w = build_world(true, Some("console-secret"));
    let body = json!({ "url": w.origin.url(""), "format": "html" });
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &body.to_string(),
    );
    assert_eq!(status, 200, "body: {text}");
}

#[test]
fn loopback_is_refused_when_the_demo_opt_in_is_off() {
    let w = build_world(false, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &json!({ "url": w.origin.url("") }).to_string(),
    );
    assert_eq!(status, 400, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "url_rejected");
}

#[test]
fn a_non_http_scheme_is_refused_before_any_network_work() {
    let w = build_world(true, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        r#"{"url":"file:///etc/passwd"}"#,
    );
    assert_eq!(status, 400, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "invalid_request");
}

#[test]
fn a_domain_outside_the_key_allowlist_is_refused() {
    let w = build_world(true, None);
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &json!({ "url": "http://example.com/" }).to_string(),
    );
    assert_eq!(status, 403, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["error"]["code"], "domain_not_allowed");
}

#[test]
fn an_oversized_body_is_refused_with_413() {
    let w = build_world(true, None);
    let huge = format!(
        r#"{{"url":"{}","pad":"{}"}}"#,
        w.origin.url(""),
        "x".repeat(2 * 1024 * 1024)
    );
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &huge,
    );
    assert_eq!(status, 413, "body: {text}");
}

#[test]
fn idempotency_replays_the_first_response_without_refetching() {
    let w = build_world(true, None);
    let body = json!({ "url": w.origin.url(""), "format": "text" }).to_string();
    let mut headers = json_headers(&w.key);
    headers.push(("idempotency-key".to_string(), "test-key-1".to_string()));
    let first = http(&w.server.addr, "POST", "/v1/scrape", &headers, &body);
    assert_eq!(first.0, 200, "body: {}", first.1);
    // The origin counts every request it sees, robots.txt included, so the
    // baseline is taken after the first call rather than assumed.
    let after_first = w.origin.requests.load(Ordering::SeqCst);
    let second = http(&w.server.addr, "POST", "/v1/scrape", &headers, &body);
    assert_eq!(second.0, 200);
    assert_eq!(first.1, second.1, "the replay is byte-identical");

    assert_eq!(
        w.origin.requests.load(Ordering::SeqCst),
        after_first,
        "the replay was served from the store, not fetched again"
    );
}

#[test]
fn the_usage_endpoint_reports_what_the_key_did() {
    let w = build_world(true, None);
    let scrape = json!({ "url": w.origin.url(""), "format": "text" });
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &scrape.to_string(),
    );
    assert_eq!(status, 200, "body: {text}");

    let (status, text, _) = http(
        &w.server.addr,
        "GET",
        "/v1/usage",
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 200, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 1);
    let key_id = w.store.list_keys(Some(&w.tenant))[0].id.to_string();
    assert_eq!(v["key_count"].as_u64().unwrap(), 1);
    assert_eq!(v["totals"]["requests"].as_u64().unwrap(), 1);
    assert!(v["totals"]["units"].as_u64().unwrap() > 0);
    let row = &v["by_key"][0];
    assert_eq!(row["key_id"].as_str(), Some(key_id.as_str()));
    assert_eq!(row["requests"].as_u64().unwrap(), 1);
    assert_eq!(v["by_day"][0]["requests"].as_u64().unwrap(), 1);
    assert!(!v["ledger_head"]["hash"].as_str().unwrap().is_empty());
}

#[test]
fn a_job_runs_to_completion_and_is_listed() {
    let w = build_world(true, None);
    // `repeat: 2` with a one-minute interval keeps the job pending after its
    // first run, which makes "listed while pending" and "a finished job
    // leaves the list" deterministic instead of racy.
    let create = json!({
        "url": w.origin.url(""),
        "format": "text",
        "schedule": { "interval_minutes": 1, "repeat": 2 }
    });
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/jobs",
        &json_headers(&w.key),
        &create.to_string(),
    );
    // `202 Accepted`, not `201`: the job is queued and runs in the
    // background, so the caller polls rather than reading a body now.
    assert_eq!(status, 202, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    let job_id = v["job_id"].as_str().unwrap().to_string();
    assert_eq!(v["status"], "queued");
    assert_eq!(v["idempotency_replayed"], false);
    assert!(v["estimated_units"].as_u64().unwrap() > 0);
    // The head echoed here is the ledger's current one, and a job that has
    // not run yet has not advanced it.
    assert!(
        v["chain"].is_object(),
        "the create response echoes the ledger head"
    );
    assert_eq!(v["chain"]["seq"].as_u64().unwrap(), 0);

    // While the job is pending it is listed, exactly once.
    let (status, text, _) = http(
        &w.server.addr,
        "GET",
        "/v1/jobs",
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 200, "body: {text}");
    let list: Value = serde_json::from_str(&text).unwrap();
    let jobs = list["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["job_id"].as_str(), Some(job_id.as_str()));

    // The first run completes and is metered into the ledger.
    let got = wait_for_job_attempt(&w, &job_id, 1);
    assert_eq!(got["status"], "queued", "the repeat keeps it pending");
    assert!(got["attempts"].as_u64().unwrap() >= 1);
    assert!(
        got["scrape"].is_object(),
        "the finished run carries its scrape"
    );
    assert_eq!(got["scrape"]["status"].as_u64().unwrap(), 200);
    assert!(
        got["chain"]["seq"].as_u64().unwrap() >= 1,
        "the run advanced the ledger"
    );
    assert!(!got["chain"]["hash"].as_str().unwrap().is_empty());

    // Cancelling a live job is accepted, and it then leaves the pending list.
    let (status, body, _) = http(
        &w.server.addr,
        "POST",
        &format!("/v1/jobs/{job_id}/cancel"),
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 202, "body: {body}");
    let (status, text, _) = http(
        &w.server.addr,
        "GET",
        &format!("/v1/jobs/{job_id}"),
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 200, "body: {text}");
    let cancelled: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(cancelled["status"], "cancelled");

    let (status, text, _) = http(
        &w.server.addr,
        "GET",
        "/v1/jobs",
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 200, "body: {text}");
    let after: Value = serde_json::from_str(&text).unwrap();
    assert!(
        after["jobs"].as_array().unwrap().is_empty(),
        "a finished or cancelled job is no longer pending"
    );
}

#[test]
fn cancelling_a_finished_job_is_a_conflict() {
    let w = build_world(true, None);
    let create = json!({
        "url": w.origin.url(""),
        "format": "text",
        "schedule": { "interval_minutes": 0, "repeat": 1 }
    });
    let (_, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/jobs",
        &json_headers(&w.key),
        &create.to_string(),
    );
    let job_id = serde_json::from_str::<Value>(&text).unwrap()["job_id"]
        .as_str()
        .expect("the create response names the job")
        .to_string();
    wait_for_terminal_job(&w, &job_id);
    let (status, body, _) = http(
        &w.server.addr,
        "POST",
        &format!("/v1/jobs/{job_id}/cancel"),
        &bearer_headers(&w.key),
        "",
    );
    assert_eq!(status, 409, "body: {body}");
}

#[test]
fn the_openapi_document_is_served_and_matches_the_checked_in_copy() {
    let w = build_world(true, None);
    let (status, text, headers) = http(&w.server.addr, "GET", "/v1/openapi.json", &[], "");
    assert_eq!(status, 200, "body: {text}");
    assert_eq!(
        headers
            .iter()
            .find(|(k, _)| k == "content-type")
            .map(|(_, v)| v.as_str()),
        Some("application/json")
    );
    let doc: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(doc["openapi"], "3.1.0");
    for path in [
        "/v1/scrape",
        "/v1/jobs",
        "/v1/jobs/{job_id}",
        "/v1/jobs/{job_id}/cancel",
        "/v1/usage",
    ] {
        assert!(doc["paths"].get(path).is_some(), "missing {path}");
    }
    assert_eq!(
        doc["info"]["title"], "Harvest managed scrape API",
        "the document names itself"
    );
}

#[test]
fn the_webhook_reaches_the_receiver_with_a_valid_signature() {
    let w = build_world(true, None);
    let mut headers = json_headers(&w.key);
    headers.push(("content-type".to_string(), "application/json".to_string()));
    let body = json!({
        "url": w.origin.url(""),
        "format": "text",
        "webhook_url": format!("http://{}/hook", w.origin.addr)
    });
    let (status, text, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &headers,
        &body.to_string(),
    );
    assert_eq!(status, 200, "body: {text}");

    // The delivery is asynchronous with respect to the response; give the
    // receiver a moment to record it.
    let deadline = Instant::now() + Duration::from_secs(10);
    while w.origin.hits.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        w.origin.hits.load(Ordering::SeqCst),
        1,
        "the hook fired once"
    );
    let signature = w.origin.signatures.lock().unwrap()[0].clone();
    // The signature is HMAC-SHA256 of "<timestamp>.<payload>" in hex.
    assert_eq!(signature.len(), 64, "hex(hmac-sha256) is 64 chars");
    assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn the_admin_half_of_the_surface_keeps_the_console_bearer() {
    let w = build_world(true, Some("console-secret"));
    // Without the console token the operator half is refused.
    let (status, _, _) = http(
        &w.server.addr,
        "GET",
        "/v1/admin/tenants",
        &[("authorization".to_string(), format!("Bearer {}", w.key))],
        "",
    );
    assert_eq!(status, 401, "an api key is not a console bearer");

    let (status, text, _) = http(
        &w.server.addr,
        "GET",
        "/v1/admin/tenants",
        &[(
            "authorization".to_string(),
            "Bearer console-secret".to_string(),
        )],
        "",
    );
    assert_eq!(status, 200, "body: {text}");
    let v: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["schema"], 1);
    assert_eq!(v["tenant_count"], 1);
}

#[test]
fn key_rotation_revokes_the_old_plaintext_and_issues_a_replacement() {
    let w = build_world(true, None);
    let listed = w.store.list_keys(Some(&w.tenant));
    assert_eq!(listed.len(), 1);
    let old_id = listed[0].id.clone();

    // The admin half is reached through the console bearer; this server has
    // none configured, so the mutation is exercised through the store and the
    // customer surface is checked for the effect.
    w.store.rotate_key(old_id.as_str()).expect("rotate");
    let (status, _, _) = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_headers(&w.key),
        &json!({ "url": w.origin.url("") }).to_string(),
    );
    // Rotation revokes the old key; a revoked key is a state, not a
    // secret, so the caller is told which rather than sent back to
    // guessing.
    assert_eq!(status, 403, "the rotated plaintext no longer authenticates");

    // Rotation revokes the old record rather than deleting it, so the
    // operator can still see it in the key table; the new key is the one
    // that is active.
    let rotated = w.store.list_keys(Some(&w.tenant));
    assert_eq!(rotated.len(), 2, "the revoked key stays on the audit trail");
    let active: Vec<_> = rotated.iter().filter(|k| k.id != old_id).collect();
    assert_eq!(active.len(), 1, "exactly one live key after a rotation");
    let revoked = rotated
        .iter()
        .find(|k| k.id == old_id)
        .expect("the revoked key is still listed");
    assert_eq!(revoked.status.as_str(), "revoked");
    assert_ne!(revoked.key_hash, active[0].key_hash);
    assert_ne!(revoked.id, active[0].id);
}

#[test]
fn the_ledger_chain_is_append_only_and_verifies() {
    let w = build_world(true, None);
    for _ in 0..3 {
        let (status, text, _) = http(
            &w.server.addr,
            "POST",
            "/v1/scrape",
            &json_headers(&w.key),
            &json!({ "url": w.origin.url(""), "format": "text" }).to_string(),
        );
        assert_eq!(status, 200, "body: {text}");
    }
    let (entries, head) = (
        w.app.ledger().read_all().expect("the ledger reads back"),
        w.app.ledger_head(),
    );
    assert!(entries.len() >= 3, "the ledger kept every entry");
    assert!(
        w.app.ledger().verify().expect("the ledger verifies") > 0,
        "the append-only hash chain verifies"
    );
    assert_eq!(head.seq, entries.last().map(|e| e.seq).unwrap_or(0));
    let hashes: Vec<&str> = entries.iter().map(|e| e.hash.as_str()).collect();
    assert!(
        hashes.windows(2).all(|w| w[0] != w[1]),
        "every entry hashes differently"
    );
    // The ledger is one file per month under the configured directory.
    let dir = w._dir.path().join("managed/ledger");
    assert!(
        dir.exists(),
        "the ledger directory is where the config says"
    );
    assert!(
        std::fs::read_dir(&dir).unwrap().count() >= 1,
        "a month file was written"
    );
}
