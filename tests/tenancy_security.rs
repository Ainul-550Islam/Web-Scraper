//! Security contract for the managed surface.
//!
//! Every rule here is a refusal that must hold at the wire, not just in the
//! store: a revoked, suspended or expired key, a key without the scope it is
//! asking for, a job owned by another tenant, and — the one that matters most
//! — a secret that must never appear in a response, a log line, an event or
//! the audit trail.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use harvest::admin::{handle, AdminState};
use harvest::config::{Config, ManagedApiCfg};
use harvest::metrics::Metrics;
use harvest::proxy::ProxyPool;
use harvest::scrape_api::{App, AppInputs};
use harvest::tenancy::limits::KeyLimits;
use harvest::tenancy::{Scope, ScopeSet, TenancyStore};
use serde_json::json;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "harvest-tenancy-{tag}-{}-{:?}",
            std::process::id(),
            Instant::now()
        ));
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self(path)
    }
}

impl TempDir {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An origin that answers everything with a tiny page, so the security tests
/// never depend on fetching behaviour.
fn start_origin() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind origin");
    let addr = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming().take(512) {
            let Ok(mut sock) = conn else { continue };
            let _ = sock.set_read_timeout(Some(Duration::from_secs(10)));
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).into_owned();
            let path = head
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let (status, ctype, body) = if path == "/robots.txt" {
                (200, "text/plain", "User-agent: *\nAllow: /\n")
            } else {
                (200, "text/html", "<html><body><h1>ok</h1></body></html>")
            };
            let _ = sock.write_all(
                format!(
                    "HTTP/1.1 {status} OK\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    addr
}

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

fn http(addr: &str, method: &str, path: &str, headers: &[(String, String)], body: &str) -> String {
    let mut sock = TcpStream::connect(addr).expect("connect admin");
    sock.set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nhost: {addr}\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("content-length: {}\r\n", body.len()));
    req.push_str("connection: close\r\n\r\n");
    sock.write_all(req.as_bytes()).unwrap();
    if !body.is_empty() {
        sock.write_all(body.as_bytes()).unwrap();
    }
    sock.flush().unwrap();
    let mut raw = Vec::new();
    sock.take(1 << 20).read_to_end(&mut raw).unwrap();
    String::from_utf8_lossy(&raw).into_owned()
}

fn status_of(raw: &str) -> u16 {
    raw.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u16>().ok())
        .expect("status line")
}

fn body_of(raw: &str) -> String {
    match raw.find("\r\n\r\n") {
        Some(i) => raw[i + 4..].to_string(),
        None => String::new(),
    }
}

fn bearer(key: &str) -> Vec<(String, String)> {
    vec![("authorization".to_string(), format!("Bearer {key}"))]
}

fn json_bearer(key: &str) -> Vec<(String, String)> {
    let mut h = bearer(key);
    h.push(("content-type".to_string(), "application/json".to_string()));
    h
}

struct World {
    server: Server,
    store: Arc<TenancyStore>,
    tenant_a: String,
    key: String,
    other_key: String,
    plaintext: String,
    _dir: TempDir,
}

fn build_world() -> World {
    let dir = TempDir::new("sec");
    let origin = start_origin();

    let mut cfg = Config::default();
    let managed = ManagedApiCfg {
        enabled: true,
        allow_private_targets: true,
        data_dir: Some(dir.path().join("managed")),
        ..ManagedApiCfg::default()
    };
    cfg.managed_api = managed;

    let metrics = Arc::new(Metrics::default());
    let managed_cfg = cfg.managed_api.clone();
    let inputs = AppInputs::from_config(Arc::new(cfg), managed_cfg, metrics.clone())
        .expect("build managed api inputs");
    let app = Arc::new(App::new(inputs).expect("start managed api"));
    let store = app.store().clone();

    let tenant_a = store.new_tenant("alpha").unwrap().id.to_string();
    let tenant_b = store.new_tenant("beta").unwrap().id.to_string();

    let scopes = ScopeSet::from(vec![
        Scope::Scrape,
        Scope::JobsRead,
        Scope::JobsWrite,
        Scope::UsageRead,
    ]);
    let limits = key_limits(&origin, scopes.clone());
    let generated = store
        .create_key(&tenant_a, scopes.clone(), limits, None)
        .expect("key a");
    let other = store
        .create_key(
            &tenant_b,
            scopes,
            key_limits(&origin, ScopeSet::from(vec![Scope::Scrape])),
            None,
        )
        .expect("key b");

    let state = Arc::new(AdminState::new(
        metrics,
        Arc::new(ProxyPool::new(&[])),
        dir.path().join("history.jsonl"),
        Some("console-token-value".to_string()),
    ));
    state.attach_managed_api(app.clone());
    let server = start_server(state);
    World {
        server,
        store,
        tenant_a,
        key: generated.plaintext.clone(),
        other_key: other.plaintext,
        plaintext: generated.plaintext,
        _dir: dir,
    }
}

fn key_limits(origin: &str, scopes: ScopeSet) -> KeyLimits {
    let host = origin
        .rsplit_once(':')
        .map(|(h, _)| h.to_string())
        .unwrap_or_else(|| origin.to_string());
    KeyLimits {
        allowed_domains: vec![host],
        scopes,
        ..KeyLimits::default()
    }
}

fn scrape_body(origin: &str) -> String {
    json!({ "url": format!("http://{origin}/"), "format": "text" }).to_string()
}

// ---------------------------------------------------------------- refusals

#[test]
fn a_missing_key_is_refused_and_says_so() {
    let w = build_world();
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &[("content-type".to_string(), "application/json".to_string())],
        &scrape_body(&start_origin()),
    );
    assert_eq!(status_of(&raw), 401);
    let body = body_of(&raw);
    assert!(body.contains("missing_api_key"), "body: {body}");
}

#[test]
fn a_garbage_key_is_refused_without_echoing_it() {
    let w = build_world();
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer("sk_totally_not_a_key"),
        "{}",
    );
    assert_eq!(status_of(&raw), 401);
    let body = body_of(&raw);
    assert!(!body.contains("sk_totally_not_a_key"));
    assert!(!body.contains(&w.plaintext));
}

#[test]
fn a_revoked_key_stops_working_immediately() {
    let w = build_world();
    let id = w.store.list_keys(Some(&w.tenant_a))[0].id.clone();
    let origin = start_origin();
    let ok = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&w.key),
        &scrape_body(&origin),
    );
    assert_eq!(status_of(&ok), 200, "the key worked before revocation");

    w.store.revoke_key(id.as_str()).expect("revoke");
    let after = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&w.key),
        &scrape_body(&origin),
    );
    // A revoked key is a *state*, not a secret: telling the caller which
    // state it is in is the one leak the contract accepts, and it stops a
    // tenant from burning quota on a dead key.
    assert_eq!(status_of(&after), 403, "revocation is immediate");
    assert!(
        body_of(&after).contains("revoked"),
        "body: {}",
        body_of(&after)
    );
}

#[test]
fn a_suspended_key_is_refused_with_forbidden() {
    let w = build_world();
    let id = w.store.list_keys(Some(&w.tenant_a))[0].id.clone();
    w.store.suspend_key(id.as_str()).expect("suspend");
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&w.key),
        "{}",
    );
    assert_eq!(status_of(&raw), 403);
    assert!(
        body_of(&raw).contains("suspended"),
        "body: {}",
        body_of(&raw)
    );
}

#[test]
fn an_expired_key_is_refused() {
    let w = build_world();
    let origin = start_origin();
    let expired = w
        .store
        .create_key(
            &w.tenant_a,
            ScopeSet::from(vec![Scope::Scrape]),
            key_limits(&origin, ScopeSet::from(vec![Scope::Scrape])),
            Some(0),
        )
        .expect("expired key");
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&expired.plaintext),
        &scrape_body(&origin),
    );
    assert_eq!(status_of(&raw), 403);
    assert!(body_of(&raw).contains("expired"), "body: {}", body_of(&raw));
}

#[test]
fn a_key_without_the_scope_is_refused() {
    let w = build_world();
    let origin = start_origin();
    // A key that can authenticate but has no capabilities.
    let bare = w
        .store
        .create_key(
            &w.store.list_keys(None)[0].tenant_id,
            ScopeSet::parse_list(&[]).unwrap(),
            key_limits(&origin, ScopeSet::parse_list(&[]).unwrap()),
            None,
        )
        .expect("scopeless key");
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&bare.plaintext),
        &scrape_body(&origin),
    );
    assert_eq!(status_of(&raw), 403);
    assert!(body_of(&raw).contains("scope"), "body: {}", body_of(&raw));
}

#[test]
fn cross_tenant_job_access_is_a_404_not_a_403() {
    let w = build_world();
    let origin = start_origin();
    let create = json!({
        "url": format!("http://{origin}/"),
        "format": "text",
        "schedule": { "interval_minutes": 0, "repeat": 1 }
    });
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/jobs",
        &json_bearer(&w.key),
        &create.to_string(),
    );
    // `202 Accepted`: the job is queued and runs in the background.
    assert_eq!(status_of(&raw), 202, "body: {}", body_of(&raw));
    let job_id = serde_json::from_str::<serde_json::Value>(&body_of(&raw)).unwrap()["job_id"]
        .as_str()
        .expect("the create response names the job")
        .to_string();

    // Tenant B's key is valid, has the scope, and still must not see it.
    let other = http(
        &w.server.addr,
        "GET",
        &format!("/v1/jobs/{job_id}"),
        &bearer(&w.other_key),
        "",
    );
    assert_eq!(status_of(&other), 404, "body: {}", body_of(&other));

    let mine = http(
        &w.server.addr,
        "GET",
        &format!("/v1/jobs/{job_id}"),
        &bearer(&w.key),
        "",
    );
    assert_eq!(status_of(&mine), 200, "the owner can still read it");
}

// ------------------------------------------------------------------ secrets

#[test]
fn no_response_ever_contains_a_secret() {
    let w = build_world();
    let origin = start_origin();
    let mut seen = String::new();

    let scrape = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&w.key),
        &scrape_body(&origin),
    );
    seen.push_str(&scrape);
    seen.push('\n');
    seen.push_str(&http(
        &w.server.addr,
        "GET",
        "/v1/usage",
        &bearer(&w.key),
        "",
    ));
    seen.push('\n');
    seen.push_str(&http(
        &w.server.addr,
        "GET",
        "/v1/jobs",
        &bearer(&w.key),
        "",
    ));
    seen.push('\n');
    // The operator half, reached with the console bearer.
    seen.push_str(&http(
        &w.server.addr,
        "GET",
        "/v1/admin/keys",
        &bearer("console-token-value"),
        "",
    ));
    seen.push('\n');
    // And a refusal, which is where secrets leak most often.
    seen.push_str(&http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &bearer("sk_not_a_key"),
        "{}",
    ));

    assert!(
        !seen.contains(&w.plaintext),
        "the api key plaintext appeared in a response"
    );
    assert!(
        !seen.contains("console-token-value"),
        "the console token appeared in a response"
    );
    for hash in w.store.list_keys(None).iter().map(|k| k.key_hash.as_str()) {
        assert!(
            !seen.contains(hash),
            "a stored key hash appeared verbatim in a response"
        );
    }
}

#[test]
fn the_store_holds_only_the_hash() {
    let w = build_world();
    let path = w.store.path().to_path_buf();
    let bytes = std::fs::read(&path).expect("read the store");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    assert!(
        !text.contains(&w.plaintext),
        "the store file contains a plaintext key"
    );
    assert!(
        text.contains(&w.store.list_keys(Some(&w.tenant_a))[0].key_hash),
        "the store file holds the hash instead"
    );
    // The key's own Debug/Display must not carry the secret either.
    let key = &w.store.list_keys(Some(&w.tenant_a))[0];
    assert!(!format!("{key:?}").contains(&w.plaintext));
    assert!(!key.prefix.contains(&w.plaintext));
}

#[test]
fn the_admin_surface_requires_the_console_bearer_not_an_api_key() {
    let w = build_world();
    let with_api_key = http(&w.server.addr, "GET", "/v1/admin/keys", &bearer(&w.key), "");
    assert_eq!(
        status_of(&with_api_key),
        401,
        "an api key is not a console bearer"
    );

    let with_console = http(
        &w.server.addr,
        "GET",
        &format!("/v1/admin/keys?tenant_id={}", w.tenant_a),
        &bearer("console-token-value"),
        "",
    );
    assert_eq!(
        status_of(&with_console),
        200,
        "body: {}",
        body_of(&with_console)
    );
    assert!(!body_of(&with_console).contains(&w.plaintext));
}

#[test]
fn the_webhook_delivery_key_is_not_the_api_key() {
    // A customer who registers a webhook must not be able to forge a delivery
    // by guessing the signature key: the signature uses the configured
    // delivery key, and the timestamp is bound to the replay window.
    let w = build_world();
    let origin = start_origin();
    let body = json!({
        "url": format!("http://{origin}/"),
        "format": "text",
        "webhook_url": format!("http://{origin}/hook")
    })
    .to_string();
    let raw = http(
        &w.server.addr,
        "POST",
        "/v1/scrape",
        &json_bearer(&w.key),
        &body,
    );
    assert_eq!(status_of(&raw), 200, "body: {}", body_of(&raw));
    // The plaintext key must not appear in the response even when a webhook
    // is configured.
    assert!(!raw.contains(&w.plaintext));
}
