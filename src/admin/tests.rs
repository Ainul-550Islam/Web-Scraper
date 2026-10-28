use super::*;

use crate::config::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn test_state(history: PathBuf) -> Arc<AdminState> {
    test_state_tok(history, None)
}

fn test_state_tok(history: PathBuf, token: Option<String>) -> Arc<AdminState> {
    let cfg = Config::default();
    Arc::new(AdminState::new(
        Arc::new(Metrics::default()),
        Arc::new(ProxyPool::new(&cfg.proxies)),
        history,
        token,
    ))
}

async fn request(path: &str, state: &Arc<AdminState>) -> String {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let st = state.clone();
    let h = tokio::spawn(async move {
        let mut server = server;
        handle(&mut server, &st).await
    });
    client
        .write_all(format!("GET {path} HTTP/1.1\r\nhost: localhost\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await.unwrap();
    h.await.unwrap().unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[test]
fn auth_helpers_are_strict_but_scheme_tolerant() {
    let head = "GET /stats HTTP/1.1\r\nHost: a\r\nAuthorization: Bearer abc123\r\n\r\n";
    assert_eq!(header_value(head, "authorization"), Some("Bearer abc123"));
    assert_eq!(header_value(head, "AUTHORIZATION"), Some("Bearer abc123"));
    assert_eq!(header_value(head, "host"), Some("a"));
    assert_eq!(header_value(head, "x-nope"), None, "missing header is None");
    assert!(bearer_ok(head, "abc123"));
    assert!(bearer_ok(head, "abc123"), "exact token");
    assert!(!bearer_ok(head, "abc12"), "prefix must not pass");
    assert!(!bearer_ok(head, "abc1234"), "superstring must not pass");
    // lowercase scheme accepted, raw token accepted (common curl mistake)
    assert!(bearer_ok(
        "GET / HTTP/1.1\r\nAuthorization: bearer abc123\r\n\r\n",
        "abc123"
    ));
    assert!(bearer_ok(
        "GET / HTTP/1.1\r\nauthorization: abc123\r\n\r\n",
        "abc123"
    ));
    assert!(
        !bearer_ok("GET / HTTP/1.1\r\n\r\n", "abc123"),
        "no header => deny"
    );
    assert!(timing_eq(b"", b""));
    assert!(!timing_eq(b"a", b"aa"), "length mismatch never equal");
    assert!(timing_eq(b"same", b"same") && !timing_eq(b"same", b"SamE"));
}

#[test]
fn recent_json_is_tail_limited_and_universal() {
    let dir = std::env::temp_dir().join(format!("harvest-admin-rj-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("h.jsonl");
    std::fs::write(&p, "{\"n\":1}\n{\"n\":2}\ngarbage\n{\"n\":4}\n").unwrap();
    let got: Vec<Value> = serde_json::from_str(&recent_json(&p, 2)).unwrap();
    // limit counts RAW lines (bounded work), not valid entries: line 1
    // newest parses, line 2 is the garbage row — skipped, not fatal.
    assert_eq!(got.len(), 1, "raw-line scan window: {got:?}");
    assert_eq!(got[0]["n"], 4, "newest first");
    let got3: Vec<Value> = serde_json::from_str(&recent_json(&p, 3)).unwrap();
    assert_eq!(
        got3.len(),
        2,
        "widening the window reaches past the garbage"
    );
    assert_eq!(got3[1]["n"], 2);
    let all: Vec<Value> = serde_json::from_str(&recent_json(&p, 50)).unwrap();
    assert_eq!(all.len(), 3, "fewer lines than limit is fine");
    // missing file => empty list, never an error (idle dashboard)
    assert_eq!(recent_json(&dir.join("nope.jsonl"), 5), "[]");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dashboard_advertises_pipeline_counters() {
    // The one-file dashboard pulls a fixed key list; new /stats keys
    // must be added there or operators fly blind on the quarantine lane.
    let html = dashboard_html();
    for needle in ["records_quarantined", "schema_drifts", "jobs_completed"] {
        assert!(
            html.contains(needle),
            "dashboard JS key list lacks {needle}"
        );
    }
}

#[tokio::test]
async fn bearer_auth_gates_all_endpoints() {
    let dir = std::env::temp_dir().join(format!("harvest-auth-{}", std::process::id()));
    let st = test_state_tok(dir.join("missing.jsonl"), Some("sekrit-token".into()));

    // no header => 401 on every route, incl. /metrics and /health
    let m = request("/metrics", &st).await;
    assert!(m.starts_with("HTTP/1.1 401 Unauthorized"), "{m}");
    assert!(m.contains("www-authenticate: Bearer"));
    let h = request("/health", &st).await;
    assert!(
        h.starts_with("HTTP/1.1 200"),
        "liveness probe stays auth-exempt: {h}"
    );
    assert!(
        !h.contains("sekrit-token"),
        "token must never appear in a response"
    );
    // ...and the probe leaks nothing beyond liveness:
    assert!(
        h.contains("\"status\":\"ok\"") && !h.contains("proxies"),
        "{h}"
    );
    let s = request("/stats", &st).await;
    assert!(
        s.starts_with("HTTP/1.1 401"),
        "everything else fails closed: {s}"
    );

    // wrong token => 401
    let (mut client, server) = tokio::io::duplex(8192);
    let st2 = st.clone();
    let hnd = tokio::spawn(async move {
        let mut s = server;
        handle(&mut s, &st2).await
    });
    client
        .write_all(
            b"GET /metrics HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer nope\r\n\r\n",
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let _ = hnd.await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 401"));

    // right token => 200
    let (mut client, server) = tokio::io::duplex(8192);
    let st3 = st.clone();
    let hnd = tokio::spawn(async move {
        let mut s = server;
        handle(&mut s, &st3).await
    });
    client
        .write_all(
            b"GET /metrics HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer sekrit-token\r\n\r\n",
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    let _ = hnd.await;
    let txt = String::from_utf8_lossy(&out).into_owned();
    assert!(txt.starts_with("HTTP/1.1 200 OK"), "{txt}");
    assert!(txt.contains("harvest_requests_total"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn routes_ok() {
    let dir = std::env::temp_dir().join(format!("harvest-admin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let hist = dir.join("runs.jsonl");
    std::fs::write(
        &hist,
        "{\"run\":\"r1\",\"ts\":1,\"status\":\"ok\",\"duration_ms\":9,\"config_sha256\":\"deadbeefcafe\",\"summary\":{\"pages_fetched\":3,\"written\":2,\"blocked\":0}}\n",
    )
    .unwrap();

    let st = test_state(hist);
    st.metrics().record_written();
    st.metrics().request_blocked();

    let m = request("/metrics", &st).await;
    assert!(m.starts_with("HTTP/1.1 200 OK"));
    assert!(m.contains("harvest_records_written 1"), "{m}");

    let h = request("/health", &st).await;
    assert!(
        h.contains("\"status\":\"ok\"") && h.contains("\"running\":false"),
        "{h}"
    );

    let s = request("/stats", &st).await;
    assert!(s.contains("\"proxies\"") && s.contains("\"running\""));

    let r = request("/recent", &st).await;
    assert!(r.contains("deadbeefcafe"), "{r}");
    assert!(r.contains("pages_fetched"));

    let p = request("/policy", &st).await;
    assert!(p.contains("not implemented, by design"));

    let d = request("/", &st).await;
    assert!(d.starts_with("HTTP/1.1 200 OK") && d.contains("harvest"));
    assert!(d.contains("text/html; charset=utf-8"));

    let n = request("/nope", &st).await;
    assert!(n.starts_with("HTTP/1.1 404 Not Found"));

    // missing history file => empty list, no crash
    let empty = test_state(dir.join("missing.jsonl"));
    let r2 = request("/recent", &empty).await;
    assert!(r2.ends_with("[]"), "{r2}");
    let _ = std::fs::remove_dir_all(&dir);
}

async fn request_full(state: &Arc<AdminState>, raw_request: &[u8]) -> String {
    let (mut client, server) = tokio::io::duplex(64 * 1024);
    let st = state.clone();
    let h = tokio::spawn(async move {
        let mut server = server;
        handle(&mut server, &st).await
    });
    client.write_all(raw_request).await.unwrap();
    let mut buf = Vec::new();
    client.read_to_end(&mut buf).await.unwrap();
    h.await.unwrap().unwrap();
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn correlation_ids_echo_or_generate() {
    let st = test_state(PathBuf::from("/nonexistent.jsonl"));
    let r = request_full(
        &st,
        b"GET /health HTTP/1.1\r\nhost: localhost\r\nx-request-id: job-7.trace\r\n\r\n",
    )
    .await;
    assert!(
        r.contains("x-request-id: job-7.trace"),
        "well-formed incoming id must be echoed: {r}"
    );
    let r = request("/stats", &st).await;
    let line = r
        .lines()
        .find(|l| l.starts_with("x-request-id:"))
        .expect("generated id header present");
    let id = line.trim()["x-request-id: ".len()..].to_string();
    assert!(
        CorrelationId::valid(&id),
        "generated id is well-formed: {id}"
    );
    // and the error envelope carries a correlation id too
    let r = request("/runs?limit=x", &st).await;
    assert!(r.starts_with("HTTP/1.1 400 Bad Request"), "{r}");
    assert!(r.contains("request_id"));
    assert!(
        r.contains("x-request-id: req-"),
        "generated id echoed in header"
    );
}

#[tokio::test]
async fn response_safety_headers_everywhere() {
    let st = test_state_tok(PathBuf::from("/nope.jsonl"), Some("tok".into()));
    let r = request_full(
        &st,
        b"GET /stats HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer tok\r\n\r\n",
    )
    .await;
    for hdr in [
        "content-type: application/json",
        "x-content-type-options: nosniff",
        "cache-control: no-store",
        "x-frame-options: DENY",
        "connection: close",
    ] {
        assert!(r.contains(hdr), "missing {hdr}:\n{r}");
    }
    // dashboard gets a CSP locking itself to same-origin fetches
    let d = request_full(
        &st,
        b"GET / HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer tok\r\n\r\n",
    )
    .await;
    assert!(
        d.contains("content-security-policy: default-src 'none'"),
        "{d}"
    );
    // request line without a target -> 400 before any state access
    let m = request_full(&st, b"HELLO\r\n\r\n").await;
    assert!(m.starts_with("HTTP/1.1 400 Bad Request"), "{m}");
    assert!(m.contains("malformed request line"));
    // query strings over the bound are rejected (400), not processed
    let big = format!(
        "GET /stats?{} HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer tok\r\n\r\n",
        "a=1&".repeat(100)
    );
    let r = request_full(&st, big.as_bytes()).await;
    assert!(r.starts_with("HTTP/1.1 400"), "{r}");
}

#[tokio::test]
async fn method_routing_is_read_mostly() {
    let st = test_state(PathBuf::from("/nope.jsonl"));
    // non-GET on a read route: 405 with the stable JSON error shape
    let r = request_full(&st, b"DELETE /stats HTTP/1.1\r\nhost: localhost\r\n\r\n").await;
    assert!(r.starts_with("HTTP/1.1 405 Method Not Allowed"), "{r}");
    assert!(r.contains("\"code\":\"method_not_allowed\""));
    // The 405 text describes this method and route, not a stale mutation list.
    assert!(r.contains("DELETE is not supported on /stats."));
    // POST to a GET route is also 405: mutations are limited to explicit POST routes.
    let r = request_full(
        &st,
        b"POST /stats HTTP/1.1\r\nhost: localhost\r\ncontent-length: 0\r\n\r\n",
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 405"), "{r}");
    // body over the hard cap: 413 before the body is read
    let r = request_full(
        &st,
        b"POST /ops/note HTTP/1.1\r\nhost: localhost\r\ncontent-length: 99999\r\n\r\n",
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 413 Payload Too Large"), "{r}");
    assert!(r.contains("\"payload_too_large\""));
}

#[test]
fn artifact_download_gate_covers_the_registered_download_route() {
    let st = test_state(PathBuf::from("/nope.jsonl"));
    assert_eq!(route_shape("/artifacts/art-1"), "/artifacts/:id");

    let first = acquire_download_slot(&st, route_shape("/artifacts/art-1"))
        .unwrap()
        .expect("first download slot");
    let second = acquire_download_slot(&st, route_shape("/artifacts/art-2"))
        .unwrap()
        .expect("second download slot");
    assert!(acquire_download_slot(&st, "/artifacts/:id").is_err());
    assert!(acquire_download_slot(&st, "/control/artifacts/:id")
        .unwrap()
        .is_none());

    drop(first);
    assert!(acquire_download_slot(&st, "/artifacts/:id").is_ok());
    drop(second);
}

#[tokio::test]
async fn ops_note_alert_ack_and_audit_flow() {
    let st = test_state(PathBuf::from("/nope.jsonl"));
    // fire a run_error into the state's ledger so ack has something live
    let cfg = Config::default();
    let fired = crate::alerts::evaluate_detailed(
        &{
            let mut c = cfg.clone();
            c.alerts.webhook_url = Some("http://127.0.0.1:1/h".into());
            c.alerts.events = vec!["run_error".into()];
            c
        },
        "error",
        &crate::engine::Summary::default(),
    );
    st.alerts_ledger().note(&cfg, &fired);
    // note an operator event
    let r = request_full(
        &st,
        b"POST /ops/note HTTP/1.1\r\nhost: localhost\r\ncontent-length: 21\r\n\r\n{\"note\":\"restart ok\"}",
    )
    .await;
    assert!(r.starts_with("HTTP/1.1 200"), "{r}");
    // ack the live event (POST-only mutating route)
    let r = request_full(
        &st,
        b"POST /alerts/ack?event=run_error HTTP/1.1\r\nhost: localhost\r\n\r\n",
    )
    .await;
    assert!(r.contains("\"acked\":1"), "{r}");
    assert!(r.contains("\"result\":\"ok\""));
    // audit shows BOTH actions, newest first, with correlation ids
    let a = request("/audit", &st).await;
    let j: Value = serde_json::from_str(a.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let items = j["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{j}");
    assert_eq!(items[0]["action"], "alerts.ack");
    assert_eq!(items[1]["action"], "ops.note");
    assert!(items[0]["corr"].as_str().unwrap().starts_with("req-"));
    // ops ring saw both, and the ack detail is safe metadata only
    let o = request("/ops", &st).await;
    let oj: Value = serde_json::from_str(o.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(oj["items"][0]["kind"], "alerts.ack");
    assert_eq!(oj["items"][0]["detail"]["event"], "run_error");
    // acking something unknown reports honestly, still 200
    let r = request_full(
        &st,
        b"POST /alerts/ack?event=not_a_real_event HTTP/1.1\r\nhost: localhost\r\n\r\n",
    )
    .await;
    assert!(r.contains("\"result\":\"no-live-entry\""), "{r}");
}

#[tokio::test]
async fn runs_endpoint_pages_filters_and_leaks_no_paths() {
    let dir = std::env::temp_dir().join(format!("harvest-runs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let hist = dir.join("runs.jsonl");
    let mut lines = String::new();
    for i in 0..30 {
        let status = if i % 10 == 9 { "error" } else { "ok" };
        lines.push_str(&format!(
            "{{\"run\":\"r{i}\",\"ts\":{i},\"status\":\"{status}\",\"duration_ms\":{},\"config_sha256\":\"sha{i:02}\",\"summary\":{{\"written\":{}}}}}\n",
            i * 7,
            30 - i
        ));
    }
    std::fs::write(&hist, &lines).unwrap();
    let st = test_state(hist.clone());

    let get = |q: &str| {
        let st = st.clone();
        let q = q.to_string();
        async move {
            let r = request(&format!("/runs?{q}"), &st).await;
            serde_json::from_str::<Value>(r.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        }
    };
    let all = get("limit=1000").await;
    assert_eq!(all["page"]["total"], 30, "window total");
    assert_eq!(all["page"]["limit"], 200, "clamp, never unbounded");
    assert_eq!(all["page"]["items"].as_array().unwrap().len(), 30);
    // newest ts first (default sort) — items[0] is r29
    assert_eq!(all["page"]["items"][0]["run"], "r29");
    let p1 = get("limit=10&offset=10").await;
    assert_eq!(p1["page"]["items"].as_array().unwrap().len(), 10);
    assert_eq!(p1["page"]["items"][0]["run"], "r19");
    // filter by status
    let errs = get("status=error").await;
    assert_eq!(errs["page"]["total"], 3);
    for it in errs["page"]["items"].as_array().unwrap() {
        assert_eq!(it["status"], "error");
    }
    // range + deterministic sort by written asc
    let asc = get("sort=written&order=asc&since=20&until=29").await;
    let runs: Vec<&str> = asc["page"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["run"].as_str().unwrap())
        .collect();
    assert_eq!(
        runs,
        vec!["r29", "r28", "r27", "r26", "r25", "r24", "r23", "r22", "r21", "r20"],
        "since/until are inclusive bounds"
    );
    // unknown filter value => 400 with the JSON error shape
    let r = request("/runs?status=weird", &st).await;
    assert!(r.starts_with("HTTP/1.1 400"), "{r}");
    // body never carries the (temp) directory path or a raw newline dump
    let raw = all.to_string();
    assert!(!raw.contains("harvest-runs-"), "path leak: {raw}");
    // events limit also clamps (was 512, now the shared bound)
    let e = request("/events?limit=9999", &st).await;
    let ej: Value = serde_json::from_str(e.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(ej["limit"], 200);
    assert_eq!(ej["error"], "no run active");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn config_endpoint_redacts_and_annotates() {
    let st = test_state(PathBuf::from("/nope.jsonl"));
    // without a config attached the endpoint says so (no fake data)
    let r = request("/config", &st).await;
    assert!(r.contains("\"unavailable\""), "{r}");
    let dir = std::env::temp_dir().join(format!("harvest-cfgx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("cfg.json");
    std::fs::write(
        &file,
        "{\"name\": \"redact-me\", \"admin_token\": \"super-secret-token\", \"targets\": [{\"id\": \"t\", \"start_url\": \"https://example.com/\", \"extract\": {\"n\": {\"css\": \"h1\"}}}], \"alerts\": {\"webhook_url\": \"https://hooks.slack.com/services/T000/AAA\", \"events\": []}}",
    )
    .unwrap();
    let cfg = Arc::new(Config::load(&file).unwrap());
    st.with_config(cfg.clone(), Some(file.clone()));
    let r = request("/config", &st).await;
    let body = r.split("\r\n\r\n").nth(1).unwrap();
    let j: Value = serde_json::from_str(body).unwrap();
    assert!(!body.contains("super-secret-token"), "token leak");
    assert!(!body.contains("T000"), "webhook secret leak");
    assert_eq!(j["admin_token"], "<redacted>");
    assert!(j["_sources"]["targets"] == "explicit");
    assert!(j["_sources"]["concurrency"] == "default");
    assert!(j["_sources"]["name"] == "explicit");
    assert_eq!(j["validation"], "ok");
    let _ = std::fs::remove_dir_all(&dir);
    // /config inspect is an operator action: audited
    let a = request("/audit", &st).await;
    assert!(a.contains("config.inspect"), "{a}");
}

#[tokio::test]
async fn diagnostics_and_pool_views_are_honest_when_idle() {
    let st = test_state(PathBuf::from("/nope.jsonl"));
    let r = request("/diagnostics", &st).await;
    let j: Value = serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(j["overall"], "healthy", "{j}"); // idle != broken
    let names: Vec<&str> = j["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"runtime") && names.contains(&"proxies") && names.contains(&"history"));
    // sorted by name for determinism
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "component order must be deterministic");
    // queue/workers say unavailable without pretending to be down
    for path in ["/queue", "/workers", "/failures", "/retries"] {
        let r = request(path, &st).await;
        let j: Value = serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(j["state"], "unavailable", "{path}: {j}");
    }
    // proxies: no configured proxies => exactly the synthetic direct
    // egress, unprobed; and the snapshot can never carry a proxy URL
    let p = request("/proxies", &st).await;
    let pj: Value = serde_json::from_str(p.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(pj["total"], 1);
    assert_eq!(pj["entries"][0]["state"], "unprobed");
    assert!(!p.contains("http"), "URL leak in /proxies: {p}");
    // the API-status accounting happened: 2xx counter moved
    let m = request("/metrics", &st).await;
    assert!(
        m.contains("harvest_api_requests_total{class=\"2xx\"}"),
        "{m}"
    );
    // /live and /health stay auth-exempt AND constant
    let st2 = test_state_tok(PathBuf::from("/nope.jsonl"), Some("tok".into()));
    let l = request_full(&st2, b"GET /live HTTP/1.1\r\nhost: localhost\r\n\r\n").await;
    assert!(
        l.starts_with("HTTP/1.1 200") && l.ends_with("{\"live\":true}"),
        "{l}"
    );
    let d = request_full(
        &st2,
        b"GET /diagnostics HTTP/1.1\r\nhost: localhost\r\n\r\n",
    )
    .await;
    assert!(
        d.starts_with("HTTP/1.1 401"),
        "diagnostics must NOT be exempt"
    );
}

#[test]
fn metrics_swap_shown_in_snapshot() {
    let st = test_state(PathBuf::from("/nonexistent.jsonl"));
    let newm = Arc::new(Metrics::default());
    newm.record_written();
    newm.record_written();
    st.set_metrics(newm);
    assert!(st.metrics().display_json()["records_written"] == 2);
}

#[tokio::test]
async fn audit_body_bytes_cannot_supply_an_authorization_header() {
    let st = test_state_tok(
        PathBuf::from("/no-history.jsonl"),
        Some("audit-secret-token".into()),
    );
    let r = request_full(&st, b"GET /stats HTTP/1.1\r\nhost: localhost\r\n\r\nAuthorization: Bearer audit-secret-token\r\n").await;
    assert!(r.starts_with("HTTP/1.1 401"), "a body is not a header: {r}");
}

#[tokio::test]
async fn audit_incomplete_headers_are_refused() {
    let st = test_state(PathBuf::from("/no-history.jsonl"));
    let (mut client, mut server) = tokio::io::duplex(64 * 1024);
    let st2 = st.clone();
    let h = tokio::spawn(async move { handle(&mut server, &st2).await });
    client
        .write_all(b"GET /stats HTTP/1.1\r\nhost: localhost\r\n")
        .await
        .unwrap();
    client.shutdown().await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    h.await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 400"));
}

#[tokio::test]
async fn audit_truncated_bodies_never_commit_a_mutation() {
    let st = test_state(PathBuf::from("/no-history.jsonl"));
    let (mut client, mut server) = tokio::io::duplex(64 * 1024);
    let st2 = st.clone();
    let h = tokio::spawn(async move { handle(&mut server, &st2).await });
    client.write_all(b"POST /ops/note HTTP/1.1\r\nhost: localhost\r\ncontent-length: 100\r\n\r\n{\"note\":\"must not commit\"}").await.unwrap();
    client.shutdown().await.unwrap();
    let mut out = Vec::new();
    client.read_to_end(&mut out).await.unwrap();
    h.await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 400"));
    assert!(!request("/ops", &st).await.contains("must not commit"));
}
