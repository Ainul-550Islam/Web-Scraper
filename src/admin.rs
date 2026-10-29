//! Tiny admin HTTP server. In `serve` mode it is process-lifetime so the
//! dashboard stays up between scheduled runs; in one-shot `run` mode the
//! task is aborted when the run finishes.
//!
//! * `/`           self-contained status dashboard (inline HTML, 5s refresh)
//! * `/live`       constant liveness reply (auth-exempt, like `/health`)
//! * `/health`     liveness probe: constant two-field JSON (auth-exempt)
//! * `/metrics`    Prometheus text exposition (runtime gauges appended
//!   while a run is attached)
//! * `/stats`      JSON snapshot (counters + proxy pool EWMA health)
//! * `/runtime`    engine runtime snapshot (queue, workers, breakers,
//!   accounting, event-bus health); reports `no run active` when idle
//! * `/events`     JSON tail of the structured event ring (`?limit=`,
//!   `?kind=`, `?since_seq=`)
//! * `/recent`     JSON tail of the run-history log
//! * `/jobs`       live job view: status + per-target tallies of the last
//!   recorded run
//! * `/runs`       filtered, paged, deterministically sorted history
//!   (`?limit=&offset=&status=&since=&until=&sort=&order=`)
//! * `/failures`   recent job failures (redacted at the source)
//! * `/retries`    recent retry delays + totals
//! * `/workers`    supervisor snapshot with a lifecycle state
//! * `/queue`      scheduler queue stats with a lifecycle state
//! * `/proxies`    proxy pool with bounded health classification
//! * `/pipeline`   transform/schema structure + counters (needs a run or
//!   `serve` to have attached the config)
//! * `/storage`    file-lane health for every configured sink/lane
//! * `/diagnostics` internal component health report (the extended view
//!   behind `harvest health`)
//! * `/alerts`     alert ledger: fires, notifications, acks
//! * `POST /alerts/ack` — acknowledge an event (`?event=name`)
//! * `/ops`        operator-plane event ring (lifecycle markers, acks)
//! * `/audit`      audit ledger of operator actions (safe metadata only)
//! * `POST /control/crawl`            start a bounded crawl over configured targets
//! * `POST /control/export`           start a bounded export job (any supported format)
//! * `POST /control/ai`               start a bounded AI-assistance round
//! * `GET  /control/jobs`             paged, filterable job list
//! * `GET  /control/jobs/:id`         one job: state, counters, artifacts
//! * `POST /control/jobs/:id/cancel`  cancel a queued or running job
//! * `GET  /control/artifacts/:id`    artifact metadata (never bytes, never a path)
//! * `GET  /artifacts/:id`            the artifact's bytes (authenticated)
//! * `GET  /artifacts/:id/verify`     re-verify the delivery it belongs to
//! * `/config`     redacted effective config + provenance + validation
//! * `/policy`     machine-readable stance — hand this to a client's legal team
//!
//! Hand-rolled HTTP to keep the dependency surface small; it is never part
//! of the data path.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::alerts::AlertLedger;
use crate::api::{self, history_window, ApiError, CorrelationId, ListFilter, Page, Query};
use crate::config::{Config, SinkCfg};
use crate::control::{
    validate_idempotency_key, ArtifactId, ControlCommand, ControlError, Operation,
    COMMAND_BODY_LIMIT, MAX_DOWNLOAD_BYTES,
};
use crate::download::ArtifactStore;
use crate::execution::{ExecState, ExecutionRegistry, JobFilter};
use crate::metrics::Metrics;
use crate::ops::{AuditLedger, OpsLog};
use crate::proxy::ProxyPool;
use crate::runtime::RuntimeSlot;

/// Upper bound for a POST body the admin API will read (4 KiB — the biggest
/// legitimate payload is an alert-ack JSON).
const MAX_BODY: u64 = 4096;

/// The one 401 body. Constant on purpose: a denied request learns nothing
/// about the path it asked for, the token it presented, or the token we
/// expect. Shared with [`crate::control`] so the control plane's refusals are
/// byte-identical to every other route's.
pub const UNAUTHORIZED_MESSAGE: &str =
    "authorization required: send 'Authorization: Bearer <token>'";

pub struct AdminState {
    /// Swapped per scheduled run so the dashboard always shows live counters;
    /// history on disk is the cumulative record.
    metrics: Mutex<Arc<Metrics>>,
    pool: Arc<ProxyPool>,
    history_path: PathBuf,
    running: AtomicBool,
    /// Live engine runtime (event bus, scheduler stats, breakers). Filled
    /// by the engine for the duration of a run; `serve` mode sees each run's
    /// runtime as it starts. Idle => the slot is empty and `/runtime` says
    /// so instead of 404-ing (a status endpoint should explain itself).
    runtime: Mutex<RuntimeSlot>,
    /// When set, every response requires `Authorization: Bearer <token>`.
    token: Option<String>,
    // ---- operator plane ----
    /// Effective config for config-aware diagnostics (`/pipeline`,
    /// `/storage`, `/config`). `serve`/`run` attach it; embedders may not
    /// have one, and those endpoints say so honestly.
    cfg: Mutex<Option<Arc<Config>>>,
    /// Path the config was loaded from (provenance source for `/config`).
    cfg_path: Mutex<Option<PathBuf>>,
    ops: Arc<OpsLog>,
    audit: Arc<AuditLedger>,
    alerts: Arc<AlertLedger>,
    downloads: Arc<tokio::sync::Semaphore>,
    /// Control plane: created when a config is attached, because crawl scope,
    /// the export source and the artifact root all come from it. `None` means
    /// the execution routes answer 503 instead of guessing.
    control: Mutex<Option<Arc<ExecutionRegistry>>>,
    /// Registered artifacts (downloads). Swapped together with the control
    /// plane so both always agree on the root directory.
    artifacts: Mutex<Arc<ArtifactStore>>,
    /// Admin-server start; powers the diagnostics uptime line.
    started: Instant,
    /// The managed customer API (`/v1/*`). Attached only when the config
    /// has it enabled: without it the customer routes answer 503 rather
    /// than falling through to the console router.
    managed_app: Mutex<Option<Arc<crate::scrape_api::App>>>,
}

impl AdminState {
    pub fn new(
        metrics: Arc<Metrics>,
        pool: Arc<ProxyPool>,
        history_path: PathBuf,
        token: Option<String>,
    ) -> Self {
        Self {
            metrics: Mutex::new(metrics),
            pool,
            history_path,
            running: AtomicBool::new(false),
            runtime: Mutex::new(RuntimeSlot::default()),
            token,
            cfg: Mutex::new(None),
            cfg_path: Mutex::new(None),
            ops: Arc::new(OpsLog::new()),
            audit: Arc::new(AuditLedger::new()),
            alerts: Arc::new(AlertLedger::new()),
            downloads: Arc::new(tokio::sync::Semaphore::new(2)),
            control: Mutex::new(None),
            artifacts: Mutex::new(Arc::new(ArtifactStore::new(
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            ))),
            started: Instant::now(),
            managed_app: Mutex::new(None),
        }
    }
    /// Attach the managed customer API. Built from the same config the
    /// console already holds, so the two surfaces agree on limits, pricing
    /// and the usage ledger.
    pub fn attach_managed_api(&self, app: Arc<crate::scrape_api::App>) {
        *self.managed_app.lock().unwrap_or_else(|p| p.into_inner()) = Some(app);
    }

    /// The managed customer API, when one was attached.
    pub fn managed_app(&self) -> Option<Arc<crate::scrape_api::App>> {
        self.managed_app
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Wire the shared runtime slot (used by `run`/`serve` so the engine can
    /// publish its live snapshot through this server without owning it).
    pub fn attach_runtime(&self, slot: RuntimeSlot) {
        *self.runtime.lock().unwrap_or_else(|p| p.into_inner()) = slot;
    }
    fn runtime_slot(&self) -> Option<Arc<crate::runtime::Runtime>> {
        self.runtime.lock().unwrap_or_else(|p| p.into_inner()).get()
    }
    pub fn metrics(&self) -> Arc<Metrics> {
        self.metrics.lock().unwrap().clone()
    }
    pub fn set_metrics(&self, m: Arc<Metrics>) {
        *self.metrics.lock().unwrap() = m;
    }
    pub fn set_running(&self, on: bool) {
        self.running.store(on, Ordering::Relaxed);
    }
    /// Attach the effective config (enables `/pipeline`, `/storage`,
    /// `/config`, and the run-scoped `/diagnostics` sections). `path` is
    /// only used by `/config` to diff explicit vs default values.
    pub fn with_config(&self, cfg: Arc<Config>, path: Option<PathBuf>) {
        *self.cfg.lock().unwrap_or_else(|p| p.into_inner()) = Some(cfg.clone());
        *self.cfg_path.lock().unwrap_or_else(|p| p.into_inner()) = path;
        self.attach_control_plane(cfg.clone());
        self.attach_managed_api_from_config(cfg);
    }

    /// Build and attach the managed customer API when the config enables it.
    /// A failure to build it disables the surface loudly (one warning, and
    /// `/v1/*` answers 503) instead of taking the console down with it.
    fn attach_managed_api_from_config(&self, cfg: Arc<Config>) {
        let managed = cfg.managed_api.clone();
        if !managed.enabled {
            return;
        }
        let inputs = match crate::scrape_api::AppInputs::from_config(cfg, managed, self.metrics()) {
            Ok(inputs) => inputs,
            Err(e) => {
                tracing::warn!(err = %e, "managed api unavailable: cannot open its state");
                return;
            }
        };
        match crate::scrape_api::App::new(inputs) {
            Ok(app) => self.attach_managed_api(Arc::new(app)),
            Err(e) => tracing::warn!(err = %e, "managed api unavailable: cannot start it"),
        }
    }

    /// Build (or rebuild) the control plane for an effective configuration.
    /// The artifact root is the directory the config already writes to, so a
    /// control-plane delivery never introduces a new writable location.
    fn attach_control_plane(&self, cfg: Arc<Config>) {
        let root = match control_root(&cfg) {
            Ok(root) => root,
            Err(_) => {
                tracing::warn!(
                    "control storage is unavailable or unsafe; mutation jobs are disabled"
                );
                return;
            }
        };
        let artifacts = Arc::new(ArtifactStore::new(root.clone()));
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let reg = Arc::new(ExecutionRegistry::new(
            root,
            artifacts.clone(),
            self.audit.clone(),
            self.ops.clone(),
            runtime,
        ));
        reg.set_config(cfg);
        *self.artifacts.lock().unwrap_or_else(|p| p.into_inner()) = artifacts;
        *self.control.lock().unwrap_or_else(|p| p.into_inner()) = Some(reg);
    }

    /// The control plane, when a configuration has been attached.
    pub fn control(&self) -> Option<Arc<ExecutionRegistry>> {
        self.control
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Registered artifacts.
    pub fn artifacts(&self) -> Arc<ArtifactStore> {
        self.artifacts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    fn config(&self) -> Option<Arc<Config>> {
        self.cfg.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
    pub fn ops(&self) -> Arc<OpsLog> {
        self.ops.clone()
    }
    pub fn audit(&self) -> Arc<AuditLedger> {
        self.audit.clone()
    }
    pub fn alerts_ledger(&self) -> Arc<AlertLedger> {
        self.alerts.clone()
    }
    /// Record an operator event; also mirrored onto the runtime event bus
    /// when a run is attached (see `crate::ops::OpsLog`).
    pub fn record_ops(&self, kind: &str, detail: Value) {
        let g = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
        self.ops.record(&g, kind, detail);
    }
}

pub async fn serve(addr: String, state: Arc<AdminState>) {
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(%addr, err = %e, "admin server could not bind; continuing without metrics endpoint");
            return;
        }
    };
    state.record_ops("serve_start", json!({"addr": addr}));
    tracing::info!(%addr, "admin server listening (dashboard on /)");
    // Bound accepted sockets as well as per-request buffers: a client that
    // never finishes headers or never reads a response must not grow tasks forever.
    let slots = Arc::new(tokio::sync::Semaphore::new(64));
    loop {
        let conn = match listener.accept().await {
            Ok(c) => c,
            Err(_) => continue,
        };
        let (mut sock, _) = conn;
        let Ok(slot) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(30),
                handle(&mut sock, &state),
            )
            .await;
        });
    }
}

/// Uniform response envelope for the router: (status, content-type, body).
type ApiResponse = (u16, &'static str, String);

fn ok_json(v: Value) -> ApiResponse {
    (200, "application/json", v.to_string())
}

fn err_api(e: ApiError, corr: &CorrelationId) -> ApiResponse {
    (
        e.status,
        "application/json",
        e.to_json(corr.as_str()).to_string(),
    )
}

fn status_line(status: u16) -> &'static str {
    match status {
        200 => "200 OK",
        201 => "201 Created",
        202 => "202 Accepted",
        204 => "204 No Content",
        400 => "400 Bad Request",
        401 => "401 Unauthorized",
        403 => "403 Forbidden",
        404 => "404 Not Found",
        405 => "405 Method Not Allowed",
        409 => "409 Conflict",
        413 => "413 Payload Too Large",
        429 => "429 Too Many Requests",
        500 => "500 Internal Server Error",
        503 => "503 Service Unavailable",
        _ => "503 Service Unavailable",
    }
}

/// Serve ONE request on a connected stream. Public for tests (unit and
/// integration) that drive the API over in-memory sockets; production entry
/// points go through [`spawn`].
#[doc(hidden)]
pub async fn handle<S>(sock: &mut S, state: &AdminState) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    // First CRLFCRLF wins. Header bytes have one absolute deadline (not a
    // timeout refreshed by every byte); body bytes can never become headers.
    let t0 = std::time::Instant::now();
    let mut buf = [0u8; 8192];
    let mut filled = 0usize;
    let boundary = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(i) = buf[..filled].windows(4).position(|w| w == b"\r\n\r\n") {
                return Ok::<usize, std::io::Error>(i + 4);
            }
            if filled == buf.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "headers too large",
                ));
            }
            let n = sock.read(&mut buf[filled..]).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "incomplete headers",
                ));
            }
            filled += n;
        }
    })
    .await;
    let boundary = match boundary {
        Ok(Ok(n)) => n,
        _ => {
            let corr = CorrelationId::new();
            let r = err_api(
                ApiError::invalid("incomplete, oversized or timed-out request headers"),
                &corr,
            );
            return respond(sock, state, r, &corr, "??", "", t0).await;
        }
    };
    let head = match std::str::from_utf8(&buf[..boundary]) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let corr = CorrelationId::new();
            let r = err_api(ApiError::invalid("invalid request headers"), &corr);
            return respond(sock, state, r, &corr, "??", "", t0).await;
        }
    };
    let mut line = head.lines().next().unwrap_or("").split_whitespace();
    let (method, target) = match (line.next(), line.next(), line.next(), line.next()) {
        (Some(m), Some(t), Some("HTTP/1.0" | "HTTP/1.1"), None) => (m.to_string(), t.to_string()),
        _ => {
            let corr = CorrelationId::new();
            let r = err_api(ApiError::invalid("malformed request line"), &corr);
            return respond(sock, state, r, &corr, "??", "", t0).await;
        }
    };
    let (route_raw, qs) = match target.split_once('?') {
        Some((r, q)) => (r.to_string(), q.to_string()),
        None => (target.clone(), String::new()),
    };
    // Illegal bytes in the request target fail before any state is touched.
    if !route_raw
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        let corr = CorrelationId::new();
        let r = err_api(ApiError::invalid("illegal byte in request target"), &corr);
        return respond(sock, state, r, &corr, &method, &route_raw, t0).await;
    }
    // Correlation id: accept only well-formed client ids, generate otherwise
    // (never echo client garbage into logs or error bodies).
    let corr = CorrelationId::from_incoming(header_value(&head, "x-request-id"));
    let route = route_raw.as_str();
    for name in [
        "content-length",
        "authorization",
        "host",
        "origin",
        "idempotency-key",
    ] {
        if head
            .lines()
            .skip(1)
            .take_while(|l| !l.is_empty())
            .filter_map(|l| l.split_once(':'))
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .count()
            > 1
        {
            let r = err_api(
                ApiError::invalid("duplicate request framing or security header"),
                &corr,
            );
            return respond(sock, state, r, &corr, &method, route, t0).await;
        }
    }
    if header_value(&head, "transfer-encoding").is_some() {
        let r = err_api(
            ApiError::invalid("transfer-encoding is unsupported; send Content-Length"),
            &corr,
        );
        return respond(sock, state, r, &corr, &method, route, t0).await;
    }

    // Bearer auth (timing-safe), fail-closed on every route EXCEPT the two
    // constant-body liveness probes: /health and /live answer fixed JSON
    // (no config, no counts, no paths), which is exactly what a container
    // orchestrator needs and safe to expose on an otherwise token-gated port.
    let open = route == "/health" || route == "/live";
    // `/v1/*` is the managed surface: the customer routes authenticate with
    // a tenant api key inside their own handlers, while `/v1/admin/*` is the
    // operator half and keeps the console bearer.
    let is_customer_route =
        managed_route(&method, route_shape(route)) && !route.starts_with("/v1/admin/");
    if !open && !is_customer_route {
        if let Some(want) = &state.token {
            if !bearer_ok(&head, want) {
                // Constant body: never echo the path, the presented header,
                // or any part of the expected token back to the client.
                let e = ApiError::unauthorized(UNAUTHORIZED_MESSAGE);
                let r = err_api(e, &corr);
                return respond(sock, state, r, &corr, &method, route, t0).await;
            }
        }
    }

    // Token-less mode is local CLI/browser development, not an open proxy.
    // A browser can rebind an attacker-controlled hostname to loopback; CORS
    // alone does not stop same-origin reads in that case.
    if !open
        && !is_customer_route
        && state.token.is_none()
        && !loopback_host(header_value(&head, "host"))
    {
        let r = err_api(
            ApiError::forbidden("token-less admin requests require a loopback Host"),
            &corr,
        );
        return respond(sock, state, r, &corr, &method, route, t0).await;
    }
    if method == "POST" && !same_origin(&head) {
        let r = err_api(
            ApiError::forbidden("cross-origin mutations are refused"),
            &corr,
        );
        return respond(sock, state, r, &corr, &method, route, t0).await;
    }

    let query = match Query::parse(&qs) {
        Ok(q) => q,
        Err(e) => {
            let r = err_api(e, &corr);
            return respond(sock, state, r, &corr, &method, route, t0).await;
        }
    };

    // Dynamic routes (`/control/jobs/:id`) are normalized to their route
    // shape before dispatch, so the handler table stays a literal match and a
    // client-supplied id is never part of a pattern.
    let route_key = route_shape(route);

    // Idempotency for mutating control-plane submissions (§29 of the control
    // plane contract): the header is validated before it is used as a key.
    let idem = match validate_idempotency_key(header_value(&head, "idempotency-key")) {
        Ok(k) => k,
        Err(e) => {
            let r = err_api(e.to_api(), &corr);
            return respond(sock, state, r, &corr, &method, route_key, t0).await;
        }
    };

    // Bodies: POST only, only on the mutating routes, hard-capped. Alert acks
    // are tiny (4 KiB); control commands get 64 KiB and no more.
    let mut body: Vec<u8> = Vec::new();
    match method.as_str() {
        "GET" if route != "/alerts/ack" && route != "/ops/note" => {}
        "POST" if route_key == "/artifacts/:id" || route_key == "/control/jobs/:id" => {
            let r = err_api(ApiError::method_not_allowed(&method, route_key), &corr);
            return respond(sock, state, r, &corr, &method, route_key, t0).await;
        }
        "POST" if route == "/alerts/ack" || route == "/ops/note" => {
            let want = match header_value(&head, "content-length") {
                Some(v) => match v.parse::<u64>() {
                    Ok(n) if n <= MAX_BODY => n,
                    Ok(_) => {
                        let r = err_api(ApiError::payload_too_large(MAX_BODY), &corr);
                        return respond(sock, state, r, &corr, &method, route, t0).await;
                    }
                    Err(_) => {
                        let r =
                            err_api(ApiError::invalid("content-length must be a number"), &corr);
                        return respond(sock, state, r, &corr, &method, route, t0).await;
                    }
                },
                None => 0,
            };
            if !read_bounded_body(sock, &mut body, want, &buf, filled).await {
                let r = err_api(
                    ApiError::invalid("incomplete or timed-out request body"),
                    &corr,
                );
                return respond(sock, state, r, &corr, &method, route_key, t0).await;
            }
        }
        "POST" if control_body_limit(route_key).is_some() => {
            let limit = control_body_limit(route_key).unwrap_or(COMMAND_BODY_LIMIT);
            let want = match header_value(&head, "content-length") {
                Some(v) => match v.parse::<u64>() {
                    Ok(n) if n <= limit => n,
                    Ok(_) => {
                        let r = err_api(ApiError::payload_too_large(limit), &corr);
                        return respond(sock, state, r, &corr, &method, route_key, t0).await;
                    }
                    Err(_) => {
                        let r =
                            err_api(ApiError::invalid("content-length must be a number"), &corr);
                        return respond(sock, state, r, &corr, &method, route_key, t0).await;
                    }
                },
                None => 0,
            };
            if !read_bounded_body(sock, &mut body, want, &buf, filled).await {
                let r = err_api(
                    ApiError::invalid("incomplete or timed-out request body"),
                    &corr,
                );
                return respond(sock, state, r, &corr, &method, route_key, t0).await;
            }
        }
        _ if route.starts_with("/v1/") => {
            // `/v1/*` answers with its own envelope, so an unknown customer
            // route must reach the managed dispatch rather than the console's
            // method table. There is no body to read for these: the shape
            // did not match a known route, so the managed handler reports it.
        }
        _ => {
            let r = err_api(ApiError::method_not_allowed(&method, route_key), &corr);
            return respond(sock, state, r, &corr, &method, route_key, t0).await;
        }
    }

    // Artifact bytes are the one non-JSON response: the payload is whatever
    // the exporter wrote, so it gets its own router table (which keeps the
    // route manifest complete) and its own reply path.
    let _download_slot = match acquire_download_slot(state, route_key) {
        Ok(slot) => slot,
        Err(e) => {
            let reply = err_api(e, &corr);
            return respond(sock, state, reply, &corr, &method, route_key, t0).await;
        }
    };
    if let Some(reply) = dispatch_raw(state, &method, route_key, route, &corr) {
        return respond_bytes(
            sock,
            state,
            reply.status,
            &reply.ctype,
            &reply.body,
            &reply.headers,
            &corr,
            &method,
            route_key,
            t0,
        )
        .await;
    }

    // The customer surface is dispatched with its own authority: it reads the
    // raw path (parameter segments are meaningful there) and answers with the
    // managed response envelope rather than the console's.
    if route.starts_with("/v1/") {
        if let Some((status, ctype, body, extra)) =
            dispatch_managed(state, &head, &method, &route_raw, &qs, &body).await
        {
            return respond_bytes(
                sock,
                state,
                status,
                ctype,
                body.as_bytes(),
                &extra,
                &corr,
                &method,
                route_key,
                t0,
            )
            .await;
        }
    }

    let resp = dispatch(
        state,
        &method,
        route_key,
        route,
        &query,
        &corr,
        &body,
        idem.as_deref(),
    );
    respond(sock, state, resp, &corr, &method, route_key, t0).await
}

/// Normalize a concrete path onto its route shape. Anything that is not a
/// known dynamic route is returned unchanged.
fn route_shape(route: &str) -> &str {
    if let Some(rest) = route.strip_prefix("/control/jobs/") {
        if rest.ends_with("/cancel") {
            return "/control/jobs/:id/cancel";
        }
        return "/control/jobs/:id";
    }
    if let Some(rest) = route.strip_prefix("/v1/admin/keys/") {
        // `/v1/admin/keys/<id>/<action>` — the id is opaque to the router.
        if let Some((id, action)) = rest.split_once('/') {
            if !id.is_empty() && !action.is_empty() {
                return match action {
                    "rotate" => "/v1/admin/keys/:id/rotate",
                    "revoke" => "/v1/admin/keys/:id/revoke",
                    "suspend" => "/v1/admin/keys/:id/suspend",
                    _ => route,
                };
            }
        }
    }
    if let Some(rest) = route.strip_prefix("/v1/jobs/") {
        if rest.ends_with("/cancel") {
            return "/v1/jobs/:id/cancel";
        }
        if !rest.is_empty() {
            return "/v1/jobs/:id";
        }
    }
    if let Some(rest) = route.strip_prefix("/control/artifacts/") {
        if !rest.is_empty() {
            return "/control/artifacts/:id";
        }
    }
    if let Some(rest) = route.strip_prefix("/artifacts/") {
        if rest.ends_with("/verify") {
            return "/artifacts/:id/verify";
        }
        return "/artifacts/:id";
    }
    route
}

fn acquire_download_slot(
    state: &AdminState,
    route_key: &str,
) -> Result<Option<tokio::sync::OwnedSemaphorePermit>, ApiError> {
    if route_key != "/artifacts/:id" {
        return Ok(None);
    }
    state
        .downloads
        .clone()
        .try_acquire_owned()
        .map(Some)
        .map_err(|_| ApiError::unavailable("artifact download capacity is busy; retry later"))
}

/// Read at most `want` body bytes: whatever the header read already
/// swallowed, then the rest under a hard deadline (no dangling-connection
/// DoS). Shared by the two body-taking route families.
async fn read_bounded_body<S>(
    sock: &mut S,
    body: &mut Vec<u8>,
    want: u64,
    buf: &[u8],
    filled: usize,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let boundary = buf[..filled]
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|i| i + 4)
            .unwrap_or(filled);
        let take = (want as usize).min(filled - boundary);
        body.extend_from_slice(&buf[boundary..boundary + take]);
        while (body.len() as u64) < want {
            let mut chunk = [0u8; 4096];
            let remaining = (want as usize - body.len()).min(chunk.len());
            match sock.read(&mut chunk[..remaining]).await {
                Ok(0) | Err(_) => return false,
                Ok(n) => body.extend_from_slice(&chunk[..n]),
            }
        }
        true
    })
    .await
    .unwrap_or(false)
}

/// Body cap for the control-plane POST routes (`None` = no body at all).
fn control_body_limit(route_key: &str) -> Option<u64> {
    match route_key {
        "/control/crawl" | "/control/export" | "/control/ai" => Some(COMMAND_BODY_LIMIT),
        "/control/jobs/:id/cancel" => Some(COMMAND_BODY_LIMIT),
        "/v1/scrape" | "/v1/jobs" | "/v1/jobs/:id/cancel" => {
            Some(crate::config::ManagedApiCfg::default().max_request_bytes)
        }
        _ => None,
    }
}

/// Write one JSON response (see [`respond_bytes`] for the header set).
async fn respond<S>(
    sock: &mut S,
    state: &AdminState,
    resp: ApiResponse,
    corr: &CorrelationId,
    method: &str,
    route: &str,
    t0: std::time::Instant,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let (status, ctype, body) = resp;
    respond_bytes(
        sock,
        state,
        status,
        ctype,
        body.as_bytes(),
        &[],
        corr,
        method,
        route,
        t0,
    )
    .await
}

/// Write one response with the full safety-header set: content-type,
/// nosniff, no-store, frame denial, correlation echo, exact length, and
/// connection: close (no keep-alive state to leak across requests).
/// `extra` is for the two artifact-download headers (content-disposition and
/// the sha256 the client can re-check).
#[allow(clippy::too_many_arguments)]
async fn respond_bytes<S>(
    sock: &mut S,
    state: &AdminState,
    status: u16,
    ctype: &str,
    body: &[u8],
    extra: &[(String, String)],
    corr: &CorrelationId,
    method: &str,
    route: &str,
    t0: std::time::Instant,
) -> std::io::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let ms = t0.elapsed().as_millis() as u64;
    state.metrics().note_api_request(status, ms);
    tracing::info!(
        target: "harvest::api",
        corr = corr.as_str(),
        method,
        route,
        status,
        ms,
        "api"
    );
    let mut hdr = format!(
        "HTTP/1.1 {}\r\ncontent-type: {ctype}\r\nx-content-type-options: nosniff\r\n\
         cache-control: no-store\r\nx-frame-options: DENY\r\nreferrer-policy: no-referrer\r\n\
         x-request-id: {}\r\n",
        status_line(status),
        corr
    );
    for (k, v) in extra {
        hdr.push_str(&format!("{k}: {v}\r\n"));
    }
    if status == 401 {
        hdr.push_str("www-authenticate: Bearer\r\n");
    }
    if ctype.starts_with("text/html") {
        hdr.push_str(
            "content-security-policy: default-src 'none'; style-src 'unsafe-inline'; \
             script-src 'unsafe-inline'; connect-src 'self'\r\n",
        );
    }
    hdr.push_str(&format!(
        "content-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    ));
    sock.write_all(hdr.as_bytes()).await?;
    sock.write_all(body).await?;
    sock.shutdown().await
}

/// The managed surface's route table, mirrored from `scrape_api::handlers`.
/// Kept as literal `("METHOD", "/path")` arms, one per line, so
/// `tests/route_contract.rs` can pin the surface against
/// `web/src/lib/apiRoutes/API_ROUTES.ts` exactly like the console router —
/// the handlers own the real matching, this is the published contract.
fn managed_route(method: &str, route: &str) -> bool {
    matches!(
        (method, route),
        ("POST", "/v1/scrape")
            | ("POST", "/v1/jobs")
            | ("GET", "/v1/jobs")
            | ("GET", "/v1/jobs/:id")
            | ("POST", "/v1/jobs/:id/cancel")
            | ("GET", "/v1/usage")
            | ("GET", "/v1/openapi.json")
            | ("POST", "/v1/admin/tenants")
            | ("GET", "/v1/admin/tenants")
            | ("POST", "/v1/admin/keys")
            | ("GET", "/v1/admin/keys")
            | ("POST", "/v1/admin/keys/:id/rotate")
            | ("POST", "/v1/admin/keys/:id/revoke")
            | ("POST", "/v1/admin/keys/:id/suspend")
            | ("GET", "/v1/admin/usage")
            | ("POST", "/v1/admin/usage/export")
    )
}

/// Hand a `/v1/*` request to the managed customer API. Returns `None` when
/// no app is attached, so the console router answers (503/404) instead of
/// silently dropping the request.
///
/// The extra headers are the managed envelope's own: a throttled reply
/// carries `Retry-After` mirrored from its `retry_after_ms`.
async fn dispatch_managed(
    state: &AdminState,
    head: &str,
    method: &str,
    raw_route: &str,
    qs: &str,
    body: &[u8],
) -> Option<(u16, &'static str, String, Vec<(String, String)>)> {
    let app = state.managed_app()?;
    let head = crate::scrape_api::handlers::Head {
        method: method.to_string(),
        path: raw_route.to_string(),
        query: qs.to_string(),
        body: body.to_vec(),
        authorization: header_value(head, "authorization").map(str::to_string),
        api_key: header_value(head, "x-api-key").map(str::to_string),
        idempotency_key: header_value(head, "idempotency-key").map(str::to_string),
        content_type: header_value(head, "content-type").map(str::to_string),
    };
    let (status, body) = crate::scrape_api::handlers::dispatch(&app, &head).await;
    let extra = managed_extra_headers(status, &body);
    Some((status, "application/json", body, extra))
}

/// `Retry-After` for a throttled managed reply.
///
/// The envelope already carries `retry_after_ms`; the HTTP layer mirrors it
/// as the standard header so a generic client can honour it without parsing
/// the body. Capped at one day so a quota reset far in the future cannot
/// produce an absurd header value.
fn managed_extra_headers(status: u16, body: &str) -> Vec<(String, String)> {
    if status != 429 {
        return Vec::new();
    }
    let Some(ms) = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("retry_after_ms"))
                .and_then(serde_json::Value::as_u64)
        })
    else {
        return Vec::new();
    };
    let secs = ms.div_ceil(1000).min(86_400);
    vec![("Retry-After".to_string(), secs.to_string())]
}

/// Route table. Every handler returns the full response triple; errors are
/// `ApiError`s mapped through `err_api` so clients see one stable JSON
/// shape. No handler may allocate from unbounded client input: list sources
/// are ring caps, bounded history tail windows, or validated page bounds.
#[allow(clippy::too_many_arguments)]
fn dispatch(
    state: &AdminState,
    method: &str,
    route: &str,
    raw_route: &str,
    query: &Query,
    corr: &CorrelationId,
    body: &[u8],
    idem: Option<&str>,
) -> ApiResponse {
    let running = state.running.load(Ordering::Relaxed);
    let metrics = state.metrics();
    match (method, route) {
        ("GET", "/") => (200, "text/html; charset=utf-8", dashboard_html()),
        ("GET", "/metrics") => (
            200,
            "text/plain; version=0.0.4",
            format!(
                "{}{}",
                metrics.render_prometheus(),
                runtime_extras(state, running)
            ),
        ),
        ("GET", "/live") => ok_json(json!({ "live": true })),
        ("GET", "/health") => ok_json(json!({ "status": "ok", "running": running })),
        ("GET", "/stats") => ok_json(json!({
            "running": running,
            "metrics": metrics.display_json(),
            "proxies": state.pool.snapshot(),
        })),
        ("GET", "/recent") => (
            200,
            "application/json",
            recent_json(&state.history_path, 20),
        ),
        ("GET", "/policy") => ok_json(json!({
            "robots_txt": "enforced (RFC-9309-ish, cached, deny => skip URL)",
            "pacing": "per-domain token bucket + min interval",
            "refusals": "401/403/429/451 => Retry-After honored, exponential backoff, circuit breaker abandons domain",
            "proxy_rotation": "health-based failover/geo only; never used to evade blocks",
            "fingerprint_spoofing": "not implemented, by design",
            "user_agent": "honest bot identity enforced by config validation"
        })),
        ("GET", "/runtime") => match state.runtime_slot() {
            Some(rt) => ok_json(rt.snapshot(running)),
            None => ok_json(json!({"error": "no run active"})),
        },
        ("GET", "/events") => {
            let limit = match query
                .parse_u64("limit")
                .map(|v| v.unwrap_or(64).clamp(1, 200) as usize)
            {
                Ok(n) => n,
                Err(e) => return err_api(e, corr),
            };
            let kind_filter = query.get("kind").map(str::to_string);
            let since = query.parse_u64("since_seq").ok().flatten().unwrap_or(0);
            match state.runtime_slot() {
                Some(rt) => {
                    let stats = rt.bus.stats();
                    // Filtered scans read a fixed 512-row window and then
                    // page from it: bounded work regardless of match rate.
                    let scan = if kind_filter.is_some() || since > 0 {
                        512usize
                    } else {
                        limit
                    };
                    let mut events: Vec<Value> = Vec::new();
                    for e in rt.bus.tail(scan).iter().rev() {
                        if events.len() >= limit {
                            break;
                        }
                        if since > 0 && e.seq <= since {
                            continue;
                        }
                        let v = serde_json::to_value(e).unwrap_or(Value::Null);
                        if let Some(k) = &kind_filter {
                            if v.get("kind").and_then(Value::as_str) != Some(k.as_str()) {
                                continue;
                            }
                        }
                        events.push(v);
                    }
                    ok_json(json!({
                        "limit": limit,
                        "newest_first": true,
                        "ring_len": stats.ring_len,
                        "dropped": stats.dropped,
                        "last_seq": stats.last_seq,
                        "events": events,
                    }))
                }
                None => ok_json(json!({"limit": limit, "error": "no run active", "events": []})),
            }
        }
        ("GET", "/jobs") => jobs_view(state, running),
        ("GET", "/runs") => runs_view(state, query, corr),
        ("GET", "/failures") => {
            let limit = match query
                .parse_u64("limit")
                .map(|v| v.unwrap_or(50).min(200) as usize)
            {
                Ok(n) => n,
                Err(e) => return err_api(e, corr),
            };
            match state.runtime_slot() {
                Some(rt) => {
                    let snap = rt.snapshot(running);
                    let all = snap
                        .pointer("/accounting/recent_failures")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let items: Vec<Value> = all.iter().rev().take(limit).cloned().collect();
                    ok_json(json!({"state": "available", "total": all.len(), "items": items}))
                }
                None => ok_json(json!({
                    "state": "unavailable",
                    "detail": "no run attached; failures are live accounting state",
                    "items": []
                })),
            }
        }
        ("GET", "/retries") => match state.runtime_slot() {
            Some(rt) => {
                let snap = rt.snapshot(running);
                let delays = snap
                    .pointer("/accounting/recent_retry_delays_ms")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                ok_json(json!({
                    "state": "available",
                    "accounting_retries": snap
                        .pointer("/accounting/retries")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    "requests_retry": metrics.display_json()["requests_retry"],
                    "recent_delays_ms": delays.iter().rev().cloned().collect::<Vec<_>>(),
                }))
            }
            None => ok_json(json!({"state": "unavailable", "recent_delays_ms": []})),
        },
        ("GET", "/workers") => {
            let health = match state.runtime_slot() {
                Some(rt) => api::WorkerHealth::from_snapshot(&rt.snapshot(running), running),
                None => api::WorkerHealth {
                    state: api::ComponentStatus::Unavailable,
                    snapshot: None,
                },
            };
            ok_json(serde_json::to_value(health).unwrap_or(Value::Null))
        }
        ("GET", "/queue") => {
            let health = match state.runtime_slot() {
                Some(rt) => api::QueueHealth::from_snapshot(&rt.snapshot(running), running),
                None => api::QueueHealth {
                    state: api::ComponentStatus::Unavailable,
                    stats: None,
                    avg_queue_wait_ms: 0.0,
                },
            };
            ok_json(serde_json::to_value(health).unwrap_or(Value::Null))
        }
        ("GET", "/proxies") => {
            let mut v =
                serde_json::to_value(api::ProxyEgressInfo::pool_info(state.pool.snapshot()))
                    .unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                // Pool-level in-flight denial count (budget observability;
                // per-entry live inflight rides on ProxySnapshot).
                obj.insert("inflight_denied".into(), state.pool.denied().into());
            }
            ok_json(v)
        }
        ("GET", "/pipeline") => match state.config() {
            Some(cfg) => ok_json(
                serde_json::to_value(api::PipelineStats::from(&cfg, &metrics))
                    .unwrap_or(Value::Null),
            ),
            None => ok_json(json!({"state": "unavailable", "detail": "no config attached"})),
        },
        ("GET", "/storage") => match state.config() {
            Some(cfg) => {
                ok_json(serde_json::to_value(api::StorageHealth::from(&cfg)).unwrap_or(Value::Null))
            }
            None => ok_json(json!({"state": "unavailable", "detail": "no config attached"})),
        },
        ("GET", "/diagnostics") => {
            let report = api::HealthReport::build(diagnostics(state, running));
            ok_json(serde_json::to_value(report).unwrap_or(Value::Null))
        }
        ("GET", "/alerts") => {
            let mut v = json!({
                "items": state.alerts.snapshot(),
                "fired": metrics.display_json()["alerts_fired"],
                "notified": metrics.display_json()["alerts_notified"],
            });
            if let Some(cfg) = state.config() {
                v["cooldown_secs"] = cfg.alerts.cooldown_secs.into();
                v["min_severity"] = cfg.alerts.min_severity.clone().into();
            }
            ok_json(v)
        }
        // ---- control plane (authenticated, bounded, audited) ----
        ("POST", "/control/crawl") => control_submit(state, Operation::Crawl, corr, idem, body),
        ("POST", "/control/export") => control_submit(state, Operation::Export, corr, idem, body),
        ("POST", "/control/ai") => control_submit(state, Operation::AiAssist, corr, idem, body),
        ("GET", "/control/jobs") => control_jobs(state, query, corr),
        ("GET", "/control/jobs/:id") => control_job(state, raw_route, "/control/jobs/", "", corr),
        ("POST", "/control/jobs/:id/cancel") => {
            control_cancel(state, raw_route, "/control/jobs/", "/cancel", corr)
        }
        ("GET", "/control/artifacts/:id") => {
            control_artifact(state, raw_route, "/control/artifacts/", "", corr)
        }
        ("GET", "/artifacts/:id/verify") => {
            artifact_verify(state, raw_route, "/artifacts/", "/verify", corr)
        }

        ("POST", "/alerts/ack") => {
            let event = match query.get("event") {
                Some(e) if e == "*" || CorrelationId::valid(e) => e.to_string(),
                Some(_) => {
                    return err_api(
                        ApiError::invalid("event name contains illegal characters"),
                        corr,
                    )
                }
                None => {
                    // or {"event": "..."} in the (already bounded) body
                    match serde_json::from_slice::<Value>(body)
                        .ok()
                        .and_then(|b| b.get("event").and_then(Value::as_str).map(str::to_string))
                    {
                        Some(e) if e.len() <= 64 && CorrelationId::valid(&e) => e,
                        _ => {
                            return err_api(
                                ApiError::invalid(
                                    "pass ?event=<name> or a JSON body with \"event\"",
                                ),
                                corr,
                            )
                        }
                    }
                }
            };
            let n = state.alerts.ack(&event);
            let result = if n > 0 { "ok" } else { "no-live-entry" };
            state.audit.note(
                "admin-api",
                "alerts.ack",
                result,
                Some(&event),
                corr.as_str(),
            );
            state.record_ops(
                "alerts.ack",
                json!({"event": event, "entries": n, "corr": corr.as_str()}),
            );
            ok_json(json!({"acked": n, "event": event, "result": result}))
        }
        ("POST", "/ops/note") => {
            let text = serde_json::from_slice::<Value>(body)
                .ok()
                .and_then(|b| b.get("note").and_then(Value::as_str).map(str::to_string));
            let Some(t) = text.filter(|t| !t.is_empty()) else {
                return err_api(
                    ApiError::invalid("JSON body with a non-empty \"note\" string is required"),
                    corr,
                );
            };
            state
                .audit
                .note("admin-api", "ops.note", "ok", None, corr.as_str());
            state.record_ops(
                "operator.note",
                json!({"note": crate::ops::sanitize(&t, 200), "corr": corr.as_str()}),
            );
            ok_json(json!({"recorded": true}))
        }
        ("GET", "/ops") => {
            let limit = match query
                .parse_u64("limit")
                .map(|v| v.unwrap_or(50).min(200) as usize)
            {
                Ok(n) => n,
                Err(e) => return err_api(e, corr),
            };
            ok_json(json!({ "items": state.ops.tail(limit) }))
        }
        ("GET", "/audit") => {
            let limit = match query
                .parse_u64("limit")
                .map(|v| v.unwrap_or(50).min(200) as usize)
            {
                Ok(n) => n,
                Err(e) => return err_api(e, corr),
            };
            ok_json(json!({ "items": state.audit.tail(limit) }))
        }
        ("GET", "/config") => match state.config() {
            Some(cfg) => {
                let path = state
                    .cfg_path
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone();
                state
                    .audit
                    .note("admin-api", "config.inspect", "ok", None, corr.as_str());
                ok_json(api::config_inspect(&cfg, path.as_deref()))
            }
            None => ok_json(json!({"state": "unavailable", "detail": "no config attached"})),
        },
        _ => (
            404,
            "text/plain",
            "see / /live /metrics /stats /runtime /events /recent /jobs /runs /failures \
             /retries /workers /queue /proxies /pipeline /storage /diagnostics /alerts \
             /ops /audit /config /control/crawl /control/export /control/ai \
             /control/jobs /control/jobs/:id /control/jobs/:id/cancel \
             /control/artifacts/:id /artifacts/:id /artifacts/:id/verify \
             /health /policy /v1/scrape /v1/jobs /v1/jobs/:id \
             /v1/jobs/:id/cancel /v1/usage /v1/openapi.json /v1/admin/*\n"
                .to_string(),
        ),
    }
}

/// Live job view: engine status from the attached runtime, per-target tallies
/// from the most recent persisted run (history is the durable record of
/// `Summary::per_target`; the live tally map is owned by the engine task).
fn jobs_view(state: &AdminState, running: bool) -> ApiResponse {
    use crate::api::{JobInfo, JobStatus, JobSummary};
    let snap = state.runtime_slot().map(|rt| rt.snapshot(running));
    let strp = |p: &str| {
        snap.as_ref()
            .and_then(|s| s.pointer(p))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let run = match strp("/engine/run").is_empty() {
        false => strp("/engine/run"),
        true => state
            .config()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "none".into()),
    };
    let status = match strp("/engine/status").as_str() {
        "running" => JobStatus::Running,
        "draining" => JobStatus::Draining,
        _ => JobStatus::Idle,
    };
    let uptime_ms = snap
        .as_ref()
        .and_then(|s| s.pointer("/engine/uptime_ms"))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| state.started.elapsed().as_millis() as u64);
    let mut jobs: Vec<JobInfo> = Vec::new();
    let hist: Vec<Value> =
        serde_json::from_str(&recent_json(&state.history_path, 1)).unwrap_or_default();
    if let Some(pt) = hist
        .first()
        .and_then(|h| h.pointer("/summary/per_target"))
        .and_then(Value::as_object)
    {
        for (tid, v) in pt {
            let g = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
            jobs.push(JobInfo {
                target: tid.clone(),
                written: g("written"),
                blocked: g("blocked"),
                failed: g("failed"),
                revalidated: g("revalidated"),
                quarantined: g("quarantined"),
            });
        }
        jobs.sort_by(|a, b| a.target.cmp(&b.target));
    }
    let summary = JobSummary {
        run,
        status,
        uptime_ms,
        jobs,
    };
    ok_json(json!({
        "note": "per-target tallies come from the most recent persisted run",
        "job": serde_json::to_value(&summary).unwrap_or(Value::Null),
    }))
}

fn runs_view(state: &AdminState, query: &Query, corr: &CorrelationId) -> ApiResponse {
    let page = match Page::from_query(query) {
        Ok(p) => p,
        Err(e) => return err_api(e, corr),
    };
    let filter = match ListFilter::from_query(query) {
        Ok(f) => f,
        Err(e) => return err_api(e, corr),
    };
    let (total_window, win) = api::runs_listing(&state.history_path, &filter, page);
    ok_json(json!({
        "window": total_window,
        "window_note": "listings scan the newest 512 history lines; `total` is within that window",
        "page": serde_json::to_value(&win).unwrap_or(Value::Null),
    }))
}

/// Extra Prometheus series that only exist while a runtime is attached.
/// Fixed names, no labels: the values are read from the live snapshot, so
/// there is no duplicated counter anywhere.
fn runtime_extras(state: &AdminState, running: bool) -> String {
    use std::fmt::Write as _;
    let Some(rt) = state.runtime_slot() else {
        return String::new();
    };
    let s = rt.snapshot(running);
    let num = |p: &str| -> u64 { s.pointer(p).and_then(Value::as_u64).unwrap_or(0) };
    let flt = |p: &str| -> f64 { s.pointer(p).and_then(Value::as_f64).unwrap_or(0.0) };
    let mut out = String::new();
    let _ = write!(
        out,
        "\n# harvest live runtime view (source: runtime snapshot; no duplicated counters)\n\
         # TYPE harvest_queue_depth gauge\nharvest_queue_depth {}\n\
         # TYPE harvest_queue_max_depth gauge\nharvest_queue_max_depth {}\n\
         # TYPE harvest_queue_inflight gauge\nharvest_queue_inflight {}\n\
         # TYPE harvest_jobs_active gauge\nharvest_jobs_active {}\n\
         # TYPE harvest_workers_registered gauge\nharvest_workers_registered {}\n\
         # TYPE harvest_workers_busy gauge\nharvest_workers_busy {}\n\
         # TYPE harvest_workers_utilization gauge\nharvest_workers_utilization {:.3}\n\
         # TYPE harvest_workers_deaths gauge\nharvest_workers_deaths {}\n\
         # TYPE harvest_breakers_nonclosed gauge\nharvest_breakers_nonclosed {}\n\
         # TYPE harvest_events_published gauge\nharvest_events_published {}\n\
         # TYPE harvest_events_dropped gauge\nharvest_events_dropped {}\n",
        num("/queue/depth"),
        num("/queue/max_depth"),
        num("/queue/inflight"),
        num("/accounting/jobs_active"),
        num("/workers/registered"),
        num("/workers/busy"),
        flt("/workers/utilization"),
        num("/workers/deaths"),
        s.get("breakers")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len()),
        num("/events/published"),
        num("/events/dropped"),
    );
    out
}

/// Component checks for `/diagnostics` (and `harvest health`). Everything
/// here reads state the process already owns - metrics, the runtime slot,
/// the proxy pool, and file metadata. Nothing opens a new connection or
/// probes a database: that is what the per-feature live tests do, not what
/// an inline health endpoint should risk on a scrape loop.
fn diagnostics(state: &AdminState, running: bool) -> Vec<api::ComponentReport> {
    use api::{ComponentReport as C, ComponentStatus::*};
    let mut comps = Vec::new();
    let mut push = |name, status, detail: Option<Value>| {
        comps.push(C {
            name,
            status,
            detail,
        })
    };
    push(
        "admin_uptime",
        Healthy,
        Some(json!({"ms": state.started.elapsed().as_millis() as u64})),
    );
    match state.config() {
        Some(cfg) => match (*cfg).clone().validate() {
            Ok(()) => push(
                "config",
                Healthy,
                Some(json!({"targets": cfg.targets.len()})),
            ),
            Err(e) => push(
                "config",
                Degraded,
                Some(json!({"error": crate::ops::sanitize(&e.to_string(), 160)})),
            ),
        },
        None => push(
            "config",
            Disabled,
            Some(json!({"detail": "no config attached"})),
        ),
    }
    let snap = state.runtime_slot().map(|rt| rt.snapshot(running));
    match &snap {
        Some(_) => push(
            "runtime",
            if running { Healthy } else { Degraded },
            Some(json!({"running": running})),
        ),
        None => push(
            "runtime",
            if running { Unavailable } else { Disabled },
            Some(
                json!({"detail": if running { "run active, no runtime attached" } else { "idle between runs" }}),
            ),
        ),
    }
    if let Some(s) = &snap {
        let q = api::QueueHealth::from_snapshot(s, running);
        push(
            "queue",
            q.state,
            Some(json!({
                "depth": q.stats.as_ref().and_then(|v| v.get("depth")).cloned().unwrap_or(Value::Null),
                "rejected": q.stats.as_ref().and_then(|v| v.get("rejected")).cloned().unwrap_or(Value::Null),
            })),
        );
        let w = api::WorkerHealth::from_snapshot(s, running);
        push(
            "workers",
            w.state,
            Some(json!({
                "busy": w.snapshot.as_ref().and_then(|v| v.get("busy")).cloned().unwrap_or(Value::Null),
                "deaths": w.snapshot.as_ref().and_then(|v| v.get("deaths")).cloned().unwrap_or(Value::Null),
            })),
        );
        let open = s
            .get("breakers")
            .and_then(Value::as_array)
            .map_or(0, |a| a.len());
        push(
            "breakers",
            if open > 0 { Degraded } else { Healthy },
            Some(json!({"nonclosed": open})),
        );
    }
    let pool = api::ProxyEgressInfo::pool_info(state.pool.snapshot());
    push(
        "proxies",
        if pool.total == 0 {
            Disabled
        } else if pool.down > 0 {
            Degraded
        } else {
            Healthy
        },
        Some(json!({
            "total": pool.total, "healthy": pool.healthy,
            "degraded": pool.degraded, "down": pool.down, "unprobed": pool.unprobed
        })),
    );
    // Storage lanes (history is always configured; the rest depend on config).
    let hist_meta = std::fs::metadata(&state.history_path).ok();
    push(
        "history",
        Healthy,
        Some(json!({
            "exists": hist_meta.is_some(),
            "bytes": hist_meta.map(|m| m.len()).unwrap_or(0),
        })),
    );
    if let Some(cfg) = state.config() {
        let st = api::StorageHealth::from(&cfg);
        let q = &st.quarantine;
        push(
            "quarantine",
            if q.configured && q.exists && q.bytes.unwrap_or(0) > 0 {
                Degraded
            } else {
                Healthy
            },
            Some(json!({"configured": q.configured, "bytes": q.bytes.unwrap_or(0)})),
        );
        let ev = &st.events;
        push(
            "events_log",
            if ev.configured && !ev.exists && running {
                Degraded
            } else if ev.configured {
                Healthy
            } else {
                Disabled
            },
            Some(json!({"configured": ev.configured, "bytes": ev.bytes.unwrap_or(0)})),
        );
        let out = &st.output;
        push(
            "output",
            if out.configured && !out.exists && running {
                Degraded
            } else {
                Healthy
            },
            Some(json!({"exists": out.exists, "kind": out.kind})),
        );
        push(
            "alerts",
            if cfg.alerts.enabled() {
                Healthy
            } else {
                Disabled
            },
            Some(json!({"entries": state.alerts.snapshot().len()})),
        );
    }
    comps
}

/// Router table for raw (non-JSON) responses. Returns `None` for every route
/// the JSON dispatcher handles. Keeping this a real `match` (rather than a
/// special case in `handle`) is what makes the route manifest complete:
/// `tests/route_contract.rs` pins the manifest against the router arms.
fn dispatch_raw(
    state: &AdminState,
    method: &str,
    route: &str,
    raw_route: &str,
    corr: &CorrelationId,
) -> Option<RawReply> {
    match (method, route) {
        ("GET", "/artifacts/:id") => {
            let id = raw_route.strip_prefix("/artifacts/").unwrap_or_default();
            Some(artifact_download(state, id, corr))
        }
        _ => None,
    }
}

// ---------------------------------------------------------- control plane

/// The directory control-plane artifacts live in: the one the attached
/// configuration already writes to (its output file's directory, or the
/// run-history directory when the sink is not a file). Nothing new is added
/// to the deployment's filesystem layout, and every registered artifact must
/// canonicalize inside it.
fn control_root(cfg: &Config) -> std::io::Result<PathBuf> {
    let candidate: Option<PathBuf> = match &cfg.output {
        SinkCfg::Jsonl { path } | SinkCfg::Csv { path } | SinkCfg::Sqlite { path, .. } => path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_path_buf()),
        SinkCfg::Dataset { dir, .. } => Some(dir.clone()),
        SinkCfg::Http { .. } | SinkCfg::Postgres { .. } | SinkCfg::Null {} => None,
    };
    let base = candidate
        .or_else(|| {
            cfg.schedule
                .history_path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(|p| p.to_path_buf())
        })
        .unwrap_or_else(|| PathBuf::from("."));
    let root = base.join(".harvest-control");
    crate::fsutil::prepare_private_directory(&root)?;
    std::fs::canonicalize(root)
}

fn control_registry(state: &AdminState) -> Result<Arc<ExecutionRegistry>, ControlError> {
    state.control().ok_or_else(|| {
        ControlError::unavailable(
            "no configuration is attached to this server; the control plane has nothing to execute",
        )
    })
}

/// One raw (non-JSON) reply: artifact bytes or a JSON error.
struct RawReply {
    status: u16,
    ctype: String,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
}

fn json_raw(status: u16, v: &Value) -> RawReply {
    RawReply {
        status,
        ctype: "application/json".to_string(),
        body: v.to_string().into_bytes(),
        headers: Vec::new(),
    }
}

/// Accept, validate and submit one control command. The route declares which
/// operation it accepts, so a crawl command can never be smuggled through the
/// export endpoint (or the other way round).
fn control_submit(
    state: &AdminState,
    expect: Operation,
    corr: &CorrelationId,
    idem: Option<&str>,
    body: &[u8],
) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => {
            state.audit.note(
                "admin-api",
                &format!("control.{}.submit", expect.as_str()),
                "unavailable",
                None,
                corr.as_str(),
            );
            return err_api(e.to_api(), corr);
        }
    };
    let cmd = match ControlCommand::from_request(
        "admin-api",
        corr.as_str(),
        body,
        idem.map(str::to_string),
    ) {
        Ok(c) => c,
        Err(e) => {
            // Refusals are audited too: "somebody tried and we said no" is
            // the record an operator needs, with no payload attached.
            state.audit.note(
                "admin-api",
                &format!("control.{}.submit", expect.as_str()),
                "rejected",
                None,
                corr.as_str(),
            );
            return err_api(e.to_api(), corr);
        }
    };
    if cmd.operation() != expect {
        let e = ControlError::invalid(format!(
            "this route accepts `{}` commands, not `{}`",
            expect.as_str(),
            cmd.operation().as_str()
        ));
        state.audit.note(
            "admin-api",
            &format!("control.{}.submit", expect.as_str()),
            "rejected",
            None,
            corr.as_str(),
        );
        return err_api(e.to_api(), corr);
    }
    match reg.submit(&cmd) {
        Ok(out) => {
            let mut v = serde_json::to_value(&out.job).unwrap_or_else(|_| json!({}));
            if let Some(obj) = v.as_object_mut() {
                obj.insert("replayed".to_string(), out.replayed.into());
            }
            (
                if out.replayed { 200 } else { 202 },
                "application/json",
                v.to_string(),
            )
        }
        Err(e) => {
            state.audit.note(
                "admin-api",
                &format!("control.{}.submit", expect.as_str()),
                "refused",
                cmd.audit_target().as_deref(),
                corr.as_str(),
            );
            err_api(e.to_api(), corr)
        }
    }
}

/// `GET /control/jobs` — paged, filtered, newest last.
fn control_jobs(state: &AdminState, query: &Query, corr: &CorrelationId) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => return err_api(e.to_api(), corr),
    };
    let limit = match query.parse_u64("limit").map(|v| v.unwrap_or(20) as usize) {
        Ok(n) => n,
        Err(e) => return err_api(e, corr),
    };
    let offset = match query.parse_u64("offset").map(|v| v.unwrap_or(0) as usize) {
        Ok(n) => n,
        Err(e) => return err_api(e, corr),
    };
    let state_filter = match query.get("state") {
        None => None,
        Some("queued") => Some(ExecState::Queued),
        Some("running") => Some(ExecState::Running),
        Some("completed") => Some(ExecState::Completed),
        Some("failed") => Some(ExecState::Failed),
        Some("cancelled") => Some(ExecState::Cancelled),
        Some(other) => {
            return err_api(
                ApiError::invalid(format!(
                    "unknown job state `{other}` (expected queued, running, completed, failed or cancelled)"
                )),
                corr,
            )
        }
    };
    let op = match query.get("op") {
        None => None,
        Some(raw) => match Operation::parse(raw) {
            Some(o) => Some(o),
            None => {
                return err_api(
                    ApiError::invalid(format!(
                        "unknown operation `{raw}` (expected crawl, export or ai)"
                    )),
                    corr,
                )
            }
        },
    };
    let (items, total) = reg.list(&JobFilter {
        state: state_filter,
        op,
        limit,
        offset,
    });
    ok_json(json!({
        "items": items,
        "total": total,
        "limit": limit.clamp(1, 200),
        "offset": offset,
        "in_flight": reg.in_flight(),
        "running": reg.running(),
        "caps": {
            "max_concurrent_jobs": crate::control::MAX_CONCURRENT_JOBS,
            "max_queue_depth": crate::control::MAX_QUEUE_DEPTH,
        },
    }))
}

/// `GET /control/jobs/:id`
fn control_job(
    state: &AdminState,
    raw_route: &str,
    prefix: &str,
    suffix: &str,
    corr: &CorrelationId,
) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => return err_api(e.to_api(), corr),
    };
    let Some(id) = path_id(raw_route, prefix, suffix) else {
        return err_api(ApiError::not_found("no such job"), corr);
    };
    match reg.get(id) {
        Some(job) => ok_json(serde_json::to_value(&job).unwrap_or_else(|_| json!({}))),
        None => err_api(ApiError::not_found("no such job"), corr),
    }
}

/// `POST /control/jobs/:id/cancel`
fn control_cancel(
    state: &AdminState,
    raw_route: &str,
    prefix: &str,
    suffix: &str,
    corr: &CorrelationId,
) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => return err_api(e.to_api(), corr),
    };
    let Some(id) = path_id(raw_route, prefix, suffix) else {
        return err_api(ApiError::not_found("no such job"), corr);
    };
    match reg.cancel(id, "admin-api") {
        Ok(job) => {
            state.record_ops("control.cancel", json!({ "job": id, "state": job.state }));
            ok_json(serde_json::to_value(&job).unwrap_or_else(|_| json!({})))
        }
        Err(e) => {
            state.audit.note(
                "admin-api",
                "control.cancel",
                "rejected",
                Some(id),
                corr.as_str(),
            );
            err_api(e.to_api(), corr)
        }
    }
}

/// `GET /control/artifacts/:id` — metadata only. Never bytes, never a path.
fn control_artifact(
    state: &AdminState,
    raw_route: &str,
    prefix: &str,
    suffix: &str,
    corr: &CorrelationId,
) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => return err_api(e.to_api(), corr),
    };
    let Some(id) = path_id(raw_route, prefix, suffix) else {
        return err_api(ApiError::not_found("no such artifact"), corr);
    };
    match reg.inspect(id) {
        Ok(v) => ok_json(v),
        Err(e) => err_api(e.to_api(), corr),
    }
}

/// `GET /artifacts/:id/verify` — re-run the manifest check for the delivery
/// the artifact belongs to, and update the artifacts' downloadable state.
fn artifact_verify(
    state: &AdminState,
    raw_route: &str,
    prefix: &str,
    suffix: &str,
    corr: &CorrelationId,
) -> ApiResponse {
    let reg = match control_registry(state) {
        Ok(r) => r,
        Err(e) => return err_api(e.to_api(), corr),
    };
    let Some(id) = path_id(raw_route, prefix, suffix) else {
        return err_api(ApiError::not_found("no such artifact"), corr);
    };
    match reg.verify(None, Some(id)) {
        Ok(v) => {
            state.audit.note(
                "admin-api",
                "artifact.verify",
                v["state"].as_str().unwrap_or("unknown"),
                Some(id),
                corr.as_str(),
            );
            ok_json(v)
        }
        Err(e) => err_api(e.to_api(), corr),
    }
}

/// Pull `:id` out of a concrete path. Returns `None` for an empty segment or
/// a path that does not start with the expected prefix, so callers answer 404
/// instead of indexing.
fn path_id<'a>(route: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let rest = route.strip_prefix(prefix)?;
    let rest = rest.strip_suffix(suffix).unwrap_or(rest);
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// `GET /artifacts/:id` — the bytes. Every guard is in
/// [`crate::download::ArtifactStore::open_bounded`]; this function only maps
/// the outcome onto a reply and writes the audit line.
fn artifact_download(state: &AdminState, id: &str, corr: &CorrelationId) -> RawReply {
    if ArtifactId::parse(id).is_none() {
        state.audit.note(
            "admin-api",
            "artifact.download",
            "not_found",
            None,
            corr.as_str(),
        );
        return json_raw(
            404,
            &ApiError::not_found("no such artifact").to_json(corr.as_str()),
        );
    }
    let store = state.artifacts();
    let opened = match store.open_bounded(id, MAX_DOWNLOAD_BYTES) {
        Ok(o) => o,
        Err(e) => {
            state.audit.note(
                "admin-api",
                "artifact.download",
                if e.status() == 404 {
                    "not_found"
                } else {
                    "refused"
                },
                Some(id),
                corr.as_str(),
            );
            return json_raw(
                e.status(),
                &e.to_control_error().to_api().to_json(corr.as_str()),
            );
        }
    };
    let body = match opened.read() {
        Ok(b) => b,
        Err(e) => {
            state.audit.note(
                "admin-api",
                "artifact.download",
                "refused",
                Some(id),
                corr.as_str(),
            );
            return json_raw(
                e.status(),
                &e.to_control_error().to_api().to_json(corr.as_str()),
            );
        }
    };
    state.audit.note(
        "admin-api",
        "artifact.download",
        "ok",
        Some(id),
        corr.as_str(),
    );
    RawReply {
        status: 200,
        ctype: opened.content_type().to_string(),
        headers: vec![
            (
                "content-disposition".to_string(),
                opened.content_disposition(),
            ),
            ("x-artifact-id".to_string(), id.to_string()),
            (
                "x-artifact-sha256".to_string(),
                opened.record.sha256.clone(),
            ),
            ("x-artifact-state".to_string(), "ready".to_string()),
        ],
        body,
    }
}

fn header_value<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines()
        .skip(1)
        .take_while(|l| !l.is_empty())
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            if k.trim().eq_ignore_ascii_case(name) {
                Some(v.trim())
            } else {
                None
            }
        })
}

fn loopback_host(host: Option<&str>) -> bool {
    host.and_then(|h| url::Url::parse(&format!("http://{h}")).ok())
        .and_then(|u| u.host_str().map(|h| h.trim_matches(['[', ']']).to_string()))
        .is_some_and(|h| {
            h.eq_ignore_ascii_case("localhost")
                || h.parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        })
}

fn same_origin(head: &str) -> bool {
    // CLI requests have no Origin. Browser requests must use exactly the
    // public origin preserved by our reverse proxies, not the backend name.
    let Some(origin) = header_value(head, "origin") else {
        return true;
    };
    let Some(host) = header_value(head, "host") else {
        return false;
    };
    let Ok(o) = url::Url::parse(origin) else {
        return false;
    };
    let Ok(h) = url::Url::parse(&format!("{}://{host}", o.scheme())) else {
        return false;
    };
    matches!(o.scheme(), "http" | "https")
        && o.username().is_empty()
        && o.password().is_none()
        && o.query().is_none()
        && o.fragment().is_none()
        && o.path() == "/"
        && o.origin() == h.origin()
}

fn bearer_ok(head: &str, want: &str) -> bool {
    let Some(hv) = header_value(head, "authorization") else {
        return false;
    };
    let got = hv
        .strip_prefix("Bearer ")
        .or_else(|| hv.strip_prefix("bearer "))
        .unwrap_or(hv);
    timing_eq(got.as_bytes(), want.as_bytes())
}

/// Length-leaky but content-constant-time compare: enough for bearer tokens
/// of fixed public length; avoids the `ring` dependency for this surface.
fn timing_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..n {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

fn recent_json(path: &std::path::Path, limit: usize) -> String {
    let out: Vec<Value> = history_window(path, limit).into_iter().rev().collect();
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

/// Single-file dashboard: no external assets (works air-gapped in a client VPC).
fn dashboard_html() -> String {
    r#"<!doctype html><html><head><meta charset="utf-8"><title>harvest — engine status</title>
<style>
 body{font:14px/1.5 ui-monospace,SFMono-Regular,Menlo,monospace;background:#0b1020;color:#dbe4ff;margin:0;padding:24px}
 h1{font-size:16px;margin:0 0 12px;color:#7ee0a3} h2{font-size:13px;margin:18px 0 6px;color:#8fa3d8;text-transform:uppercase;letter-spacing:.08em}
 .grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(150px,1fr));gap:8px}
 .card{background:#141b33;border:1px solid #24304f;border-radius:8px;padding:10px}
 .card b{display:block;font-size:20px;color:#fff}.card span{font-size:11px;color:#8fa3d8}
 table{border-collapse:collapse;width:100%}td,th{padding:4px 8px;border-bottom:1px solid #24304f;text-align:left;font-size:12px}
 .ok{color:#7ee0a3}.warn{color:#f5c163}.err{color:#ff7b7b}
 .badge{display:inline-block;padding:2px 8px;border-radius:999px;font-size:11px;margin-left:8px}
 .running{background:#123b26;color:#7ee0a3}.idle{background:#20263f;color:#8fa3d8}
</style></head><body>
<h1>harvest <span id=badge class="badge idle">loading</span></h1>
<h2>Run stats</h2><div id=cards class=grid>—</div>
<h2>Proxy egress health</h2><table id=proxies><tr><td>loading…</td></tr></table>
<h2>Recent runs</h2><table id=runs><tr><td>loading…</td></tr></table>
<script>
function esc(v){return String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]))}
async function j(u){const r=await fetch(u);if(!r.ok)throw new Error('HTTP '+r.status);return r.json()}
async function tick(){
 try{
  const s=await j('/stats'), m=s.metrics;
  const b=document.getElementById('badge');b.textContent=s.running?'running':'idle';b.className='badge '+(s.running?'running':'idle');
  const keys=['requests_total','requests_ok','requests_retry','requests_blocked','pages_fetched','records_extracted','records_written','records_deduped','robots_denials','domains_abandoned','jobs_completed','jobs_failed','records_quarantined','schema_drifts','pipeline_failures','storage_writes','api_requests_total','alerts_fired','alerts_notified'];
  document.getElementById('cards').innerHTML=keys.map(k=>{
    const v=m[k]||0;const hot=(k==='requests_blocked'||k==='domains_abandoned'||k==='robots_denials')&&v>0;
    return `<div class="card${hot?' warn':''}"><b>${esc(v)}</b><span>${esc(k)}</span></div>`}).join('');
  document.getElementById('proxies').innerHTML='<tr><th>id</th><th>region</th><th>requests</th><th>failures</th><th>latency ewma</th><th>success ewma</th></tr>'+
   (s.proxies||[]).map(p=>`<tr><td>${esc(p.id)}</td><td>${esc(p.region||'direct')}</td><td>${esc(p.requests)}</td><td class="${p.failures>0?'err':''}">${esc(p.failures)}</td><td>${Math.round(p.latency_ewma_ms)}ms</td><td>${(p.success_ewma*100).toFixed(1)}%</td></tr>`).join('');
  const rs=await j('/recent');
  document.getElementById('runs').innerHTML='<tr><th>when</th><th>run</th><th>status</th><th>pages</th><th>records</th><th>blocked</th><th>ms</th><th>policy sha</th></tr>'+
   (rs.map(r=>`<tr><td>${new Date((r.ts||0)*1000).toLocaleString()}</td><td>${esc(r.run)}</td><td class="${r.status==='ok'?'ok':'err'}">${esc(r.status)}</td><td>${esc(r.summary?r.summary.pages_fetched:'-')}</td><td>${esc(r.summary?r.summary.written:'-')}</td><td class="${r.summary&&r.summary.blocked>0?'warn':''}">${esc(r.summary?r.summary.blocked:'-')}</td><td>${esc(r.duration_ms??'-')}</td><td><code>${esc(String(r.config_sha256||'').slice(0,10))}</code></td></tr>`).join('')||'<tr><td>no runs recorded yet</td></tr>');
 }catch(e){const b=document.getElementById('badge');if(b){b.textContent='offline';b.className='badge idle'}}
}tick();setInterval(tick,5000)
</script></body></html>
"#
    .to_string()
}

#[cfg(test)]
mod tests;
