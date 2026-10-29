//! Request handlers for `/v1/*`.
//!
//! Two families live here. Customer endpoints authenticate with a tenant
//! api key and fail closed; operator endpoints under `/v1/admin/*` are
//! reached through the console bearer, which `admin.rs` has already
//! checked. Everything returns a `(status, body)` pair so the hand-rolled
//! router stays the single place that speaks HTTP.
//!
//! Customer endpoints do asynchronous work (network fetches, webhook
//! delivery, job execution) and the console router is asynchronous, so the
//! handlers are awaited directly. [`dispatch`] exists for callers that are
//! not inside a runtime — a CLI subcommand, a test that drives the router
//! on a plain thread — and it must be called from one, never from inside a
//! runtime: a nested `block_on` panics.

use serde_json::{json, Value};

use crate::api::ApiError;
use crate::tenancy::auth::authenticate;
use crate::tenancy::{Scope, ScopeSet};
use crate::usage::aggregate::{summarize, UsageFilter};
use crate::usage::export::{export_csv, export_jsonl, report_json};
use crate::usage::ledger::RenderMode;

use super::models::{
    CreateJobRequest, ErrorBody, ErrorResponse, JobResponse, JobState, ScrapeRequest,
    ScrapeResponse, RESPONSE_SCHEMA,
};
use super::App;

/// Everything the router hands over. Built by `admin.rs` from the request.
#[derive(Debug, Clone)]
pub struct Head {
    pub method: String,
    /// Raw request path, `/v1/...`, with parameter segments still in their
    /// `:name` form.
    pub path: String,
    /// Raw query string, without the leading `?`.
    pub query: String,
    pub body: Vec<u8>,
    pub authorization: Option<String>,
    /// Raw `X-API-Key` header. Accepted as an alternative to the bearer
    /// header; both are checked and neither is echoed back.
    pub api_key: Option<String>,
    pub idempotency_key: Option<String>,
    pub content_type: Option<String>,
}

impl Head {
    /// The segment after `/v1/`, lowercased.
    fn segment(&self) -> Option<&str> {
        self.path
            .strip_prefix("/v1/")
            .and_then(|rest| rest.split('/').next())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    }

    /// Everything after the first segment.
    fn rest(&self) -> &str {
        self.path
            .strip_prefix("/v1/")
            .and_then(|rest| rest.split_once('/'))
            .map(|(_, rest)| rest)
            .unwrap_or("")
    }
}

/// Entry point for callers outside a runtime (the CLI, a hand-driven test).
///
/// Panics if the current thread is already driving a tokio runtime; the
/// router calls [`dispatch_async`] and awaits it instead.
pub async fn dispatch(app: &App, head: &Head) -> (u16, String) {
    dispatch_async(app, head).await
}

/// Asynchronous core.
pub async fn dispatch_async(app: &App, head: &Head) -> (u16, String) {
    let method = head.method.as_str();
    match (method, head.segment()) {
        // ---- operator console (bearer already checked by admin.rs) -------
        ("POST", Some("admin")) | ("GET", Some("admin")) => admin_dispatch(app, head).await,

        // ---- the published contract -------------------------------------
        ("GET", Some("openapi.json")) => (200, super::openapi::render(app.managed())),

        // ---- customer surface (api key auth) ----------------------------
        _ => customer_async(app, head).await,
    }
}

// --------------------------------------------------------------------- auth

fn from_api_error(e: ApiError) -> (u16, String) {
    err(e.status, ErrorBody::new(e.code, e.message))
}

fn authenticate_key(app: &App, head: &Head) -> Result<crate::tenancy::Principal, (u16, String)> {
    // Both header forms are absent: name the missing credential rather than
    // reporting a generic auth failure, so a caller can tell "you did not send
    // anything" apart from "that key is not good".
    if head.authorization.is_none() && head.api_key.is_none() {
        return Err(err(
            401,
            ErrorBody::new(
                "missing_api_key",
                "send 'Authorization: Bearer <api-key>' or 'X-API-Key: <api-key>'",
            ),
        ));
    }
    authenticate(
        app.store(),
        head.authorization.as_deref(),
        head.api_key.as_deref(),
    )
    .map_err(from_api_error)
}

async fn customer_async(app: &App, head: &Head) -> (u16, String) {
    let method = head.method.as_str();
    let rest = head.rest();
    match (method, head.segment(), rest) {
        ("POST", Some("scrape"), "") => scrape_once(app, head).await,
        ("POST", Some("jobs"), "") => create_job(app, head).await,
        ("GET", Some("jobs"), "") => list_jobs(app, head).await,
        ("GET", Some("jobs"), id) => get_job(app, head, id).await,
        ("POST", Some("jobs"), id) if id.ends_with("/cancel") => cancel_job(app, head, id).await,
        ("GET", Some("usage"), "") => read_usage(app, head).await,
        _ => err(
            404,
            ErrorBody::new(
                "not_found",
                format!("no managed route for {} {}", method, head.path),
            ),
        ),
    }
}

// ---------------------------------------------------------------- /v1/scrape

async fn scrape_once(app: &App, head: &Head) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::Scrape) {
        return from_api_error(e);
    }
    let limits = principal.limits().clone();
    let tenant_id = principal.tenant_id().to_string();
    let key_id = principal.key_id().to_string();

    if let Err(e) = app.limiter().check_quota_for(&key_id, &limits) {
        app.metrics().note_rate_limited();
        app.metrics().note_quota_denied();
        return err(429, metered_error(e));
    }

    let request: ScrapeRequest = match parse_body(head, app.managed()) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if let Err(e) = crate::scrape_api::validate::validate_request(app.managed(), &request) {
        return err(400, e);
    }
    let host = host_of(&request.url);
    if !limits.allows_domain(&host) {
        return err(
            403,
            ErrorBody::new(
                "domain_not_allowed",
                format!("this key cannot scrape {host}"),
            ),
        );
    }

    // Idempotency: a repeat of the same key inside the window returns the
    // stored response, so it cannot be charged twice.
    let idempotency_key = head
        .idempotency_key
        .clone()
        .or_else(|| body_idempotency_key(head));
    if let Some(key) = &idempotency_key {
        if let Ok(Some(record)) = app.idempotency().lookup(key) {
            return (record.status, record.body);
        }
    }

    let request_id = crate::ids::CorrelationId::new().to_string();
    let webhook_url = request.webhook_url.clone();

    let permit = match app.limiter().acquire(&key_id, &limits) {
        Ok(p) => p,
        Err(e) => {
            app.metrics().note_rate_limited();
            return err(429, metered_error(e));
        }
    };

    let started = std::time::Instant::now();
    let outcome = match app
        .service()
        .run(&request, &request_id, &limits, &tenant_id, &key_id)
        .await
    {
        Ok(o) => o,
        Err(e) => {
            drop(permit);
            // A refusal is metered like any other request: the tenant asked,
            // and the site or the policy declined.
            meter(
                app,
                &tenant_id,
                &key_id,
                &request_id,
                0,
                RenderMode::None,
                false,
                request.proxy_region.clone(),
                outcome_class(e.status()),
                e.status(),
                0,
                started.elapsed().as_millis() as u64,
            );
            app.metrics().note_managed_request(e.status());
            return (e.status(), err_json(e.body()));
        }
    };
    drop(permit);

    let units = app
        .service()
        .estimate_units(&request, &limits, outcome.bytes, 0);
    app.limiter().note_usage(&key_id, units as u64);
    app.metrics().note_usage_units(units);
    app.metrics().note_managed_request(200);
    meter(
        app,
        &tenant_id,
        &key_id,
        &outcome.request_id,
        units,
        outcome.render,
        outcome.screenshot.is_some(),
        request.proxy_region.clone(),
        "ok",
        outcome.status,
        outcome.bytes,
        outcome.latency_ms,
    );

    let response = build_scrape_response(app, &request, &outcome, units);
    let status = 200;
    let body = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());

    if let Some(key) = &idempotency_key {
        if let Ok(Insert::Replayed(record)) = app.idempotency().store(key, status, &body) {
            return (record.status, record.body);
        }
    }

    if let Some(url) = webhook_url {
        deliver_webhook(
            app,
            &tenant_id,
            &key_id,
            &url,
            &response,
            &outcome.request_id,
        )
        .await;
    }
    (status, body)
}

fn build_scrape_response(
    app: &App,
    request: &ScrapeRequest,
    outcome: &super::service::ScrapeOutcome,
    units: u32,
) -> ScrapeResponse {
    use crate::scrape_api::models::OutputFormat;

    let body = match request.format {
        OutputFormat::Html | OutputFormat::Screenshot => outcome.body.clone(),
        OutputFormat::Json => outcome
            .extracted
            .clone()
            .and_then(|v| serde_json::to_string(&v).ok())
            .unwrap_or_else(|| outcome.body.clone()),
        OutputFormat::Text => crate::scrape_api::service::html_to_text(&outcome.body),
        OutputFormat::Markdown => crate::scrape_api::service::html_to_markdown(&outcome.body),
    };
    ScrapeResponse {
        schema: RESPONSE_SCHEMA,
        request_id: outcome.request_id.clone(),
        status: outcome.status,
        url: outcome.url.clone(),
        final_url: outcome.final_url.clone(),
        content_type: outcome.content_type.clone(),
        bytes: outcome.bytes,
        latency_ms: outcome.latency_ms,
        render: outcome.render.as_str().to_string(),
        format: request.format.as_str().to_string(),
        body: Some(body),
        extracted: outcome.extracted.clone(),
        records: Some(outcome.records.clone()),
        artifacts: outcome.artifacts.clone(),
        usage_units: units,
        cost: app.pricing().cost(units),
        screenshot: outcome.screenshot.clone(),
        chain: app.ledger_head(),
    }
}

fn body_idempotency_key(head: &Head) -> Option<String> {
    let parsed: Value = serde_json::from_slice(&head.body).ok()?;
    parsed
        .get("idempotency_key")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

// --------------------------------------------------------------- /v1/jobs

async fn create_job(app: &App, head: &Head) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::Scrape) {
        return from_api_error(e);
    }
    if !app.managed().jobs.enabled {
        return err(
            403,
            ErrorBody::new(
                "jobs_disabled",
                "job submission is disabled on this deployment",
            ),
        );
    }
    let limits = principal.limits().clone();
    if let Err(e) = app.limiter().check_quota_for(principal.key_id(), &limits) {
        app.metrics().note_quota_denied();
        return err(429, metered_error(e));
    }
    let request: CreateJobRequest = match parse_body(head, app.managed()) {
        Ok(r) => r,
        Err(r) => return r,
    };
    if let Err(e) = crate::scrape_api::validate::validate_request(app.managed(), &request.scrape) {
        return err(400, e);
    }
    if request.schedule.interval_minutes > app.managed().jobs.max_interval_minutes {
        return err(
            400,
            ErrorBody::new(
                "invalid_request",
                "schedule.interval_minutes exceeds the configured maximum",
            ),
        );
    }

    let job_id = format!("job-{}", crate::ids::CorrelationId::new());
    let request_id = crate::ids::CorrelationId::new().to_string();
    let job = super::jobs::JobRecord {
        id: job_id.clone(),
        request: request.scrape.clone(),
        schedule: request.schedule,
        tenant_id: principal.tenant_id().to_string(),
        key_id: principal.key_id().to_string(),
        key_hash: String::new(),
        state: JobState::Queued,
        request_id: request_id.clone(),
        created_at_ms: crate::scrape_api::time_now_ms(),
        started_at_ms: None,
        finished_at_ms: None,
        response: None,
        records: Vec::new(),
        artifacts: Vec::new(),
        error: None,
        attempts: 0,
        runs_completed: 0,
        cancelled: false,
        idempotency_key: request.scrape.idempotency_key.clone(),
    };
    if let Err(e) = app.jobs().create(job) {
        return err(500, ErrorBody::new("internal_error", format!("{e}")));
    }

    let worker = app.clone();
    let worker_job = job_id.clone();
    let worker_request = request.scrape.clone();
    let worker_tenant = principal.tenant_id().to_string();
    let worker_key = principal.key_id().to_string();
    let worker_limits = limits;
    let worker_webhook = request.scrape.webhook_url.clone();
    let worker_request_id = request_id;
    tokio::spawn(async move {
        run_job_loop(
            &worker,
            &worker_job,
            &worker_request,
            &worker_tenant,
            &worker_key,
            &worker_limits,
            &worker_webhook,
            &worker_request_id,
        )
        .await;
    });

    let body = json!({
        "schema": RESPONSE_SCHEMA,
        "job_id": job_id,
        "status": JobState::Queued.as_str(),
        "estimated_units": app.pricing().units_for(&request.scrape, 0, 0),
        "idempotency_replayed": false,
        "chain": app.ledger_head(),
    });
    (202, body.to_string())
}

/// Drive one job until it is exhausted or cancelled.
///
/// `schedule.repeat` counts the runs and `schedule.interval_minutes` is the
/// pause between them (`0` means "once", and no pause). A job cancelled
/// mid-run stops at the next boundary and keeps the state it had.
/// `repeat: 0` means "run forever", so the loop keeps going until the job is
/// cancelled rather than stopping after the first attempt.
#[allow(clippy::too_many_arguments)]
async fn run_job_loop(
    app: &App,
    job_id: &str,
    request: &ScrapeRequest,
    tenant_id: &str,
    key_id: &str,
    limits: &crate::tenancy::limits::KeyLimits,
    webhook_url: &Option<String>,
    request_id: &str,
) {
    loop {
        let Some(job) = app.jobs().get(job_id) else {
            return;
        };
        if job.cancelled || job.exhausted() {
            return;
        }
        let interval_ms = u64::from(job.schedule.interval_minutes).saturating_mul(60_000);

        run_job_once(
            app,
            job_id,
            request,
            tenant_id,
            key_id,
            limits,
            webhook_url,
            request_id,
        )
        .await;

        if app
            .jobs()
            .get(job_id)
            .is_some_and(|j| j.cancelled || j.exhausted())
        {
            return;
        }
        // A repeat with no interval would spin on the origin, so the pause
        // has a floor even when the schedule leaves it at zero.
        tokio::time::sleep(std::time::Duration::from_millis(interval_ms.max(250))).await;
    }
}

/// One attempt at a job: run it, meter it, record the outcome.
#[allow(clippy::too_many_arguments)]
async fn run_job_once(
    app: &App,
    job_id: &str,
    request: &ScrapeRequest,
    tenant_id: &str,
    key_id: &str,
    limits: &crate::tenancy::limits::KeyLimits,
    webhook_url: &Option<String>,
    request_id: &str,
) {
    let Some(job) = app.jobs().get(job_id) else {
        return;
    };
    if job.cancelled {
        return;
    }
    app.jobs()
        .transition(job_id, JobState::Queued, JobState::Running, |j| {
            j.started_at_ms = Some(crate::scrape_api::time_now_ms());
            j.attempts = j.attempts.saturating_add(1);
        });

    let started = std::time::Instant::now();
    let result = app
        .service()
        .run(request, request_id, limits, tenant_id, key_id)
        .await;
    let finished = crate::scrape_api::time_now_ms();

    match result {
        Ok(outcome) => {
            let units = app
                .service()
                .estimate_units(request, limits, outcome.bytes, 0);
            app.limiter().note_usage(key_id, units as u64);
            app.metrics().note_usage_units(units);
            app.metrics().note_managed_request(200);
            meter(
                app,
                tenant_id,
                key_id,
                &outcome.request_id,
                units,
                outcome.render,
                outcome.screenshot.is_some(),
                request.proxy_region.clone(),
                "ok",
                outcome.status,
                outcome.bytes,
                outcome.latency_ms,
            );
            let response = build_scrape_response(app, request, &outcome, units);
            let repeat = job.schedule.repeat;
            app.jobs().update(job_id, |j| {
                if j.cancelled {
                    return;
                }
                if repeat != 0 && j.runs_completed + 1 >= repeat {
                    j.state = JobState::Done;
                } else {
                    j.state = JobState::Queued;
                }
                j.response = Some(response.clone());
                j.records = outcome.records.clone();
                j.artifacts = outcome.artifacts.clone();
                j.finished_at_ms = Some(finished);
                j.runs_completed = j.runs_completed.saturating_add(1);
            });
            if let Some(url) = webhook_url {
                deliver_webhook(app, tenant_id, key_id, url, &response, job_id).await;
            }
        }
        Err(e) => {
            meter(
                app,
                tenant_id,
                key_id,
                request_id,
                0,
                RenderMode::None,
                false,
                request.proxy_region.clone(),
                outcome_class(e.status()),
                e.status(),
                0,
                started.elapsed().as_millis() as u64,
            );
            app.metrics().note_managed_request(e.status());
            app.jobs().update(job_id, |j| {
                if j.cancelled {
                    return;
                }
                j.state = JobState::Failed;
                j.error = Some(e.body());
                j.finished_at_ms = Some(finished);
            });
        }
    }
}

async fn get_job(app: &App, head: &Head, id: &str) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::JobsRead) {
        return from_api_error(e);
    }
    let Some(job) = app.jobs().get(id) else {
        return err(404, ErrorBody::new("not_found", "no job with this id"));
    };
    if job.tenant_id != principal.tenant_id() {
        // Another tenant's job is a 404, not a 403: the id must not confirm
        // that it exists.
        return err(404, ErrorBody::new("not_found", "no job with this id"));
    }
    (
        200,
        serde_json::to_string(&job_json(&job, app)).unwrap_or_default(),
    )
}

async fn list_jobs(app: &App, head: &Head) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::JobsRead) {
        return from_api_error(e);
    }
    let limit = query_usize(head, "limit")
        .unwrap_or(20)
        .min(app.managed().jobs.jobs_max_list);
    let tenant = principal.tenant_id().to_string();
    let jobs: Vec<Value> = app
        .jobs()
        .pending()
        .into_iter()
        .filter(|j| j.tenant_id == tenant)
        .take(limit)
        .map(|j| serde_json::to_value(job_json(&j, app)).unwrap_or(Value::Null))
        .collect();
    (
        200,
        json!({"schema": RESPONSE_SCHEMA, "jobs": jobs}).to_string(),
    )
}

async fn cancel_job(app: &App, head: &Head, id: &str) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::JobsWrite) {
        return from_api_error(e);
    }
    let job_id = id.strip_suffix("/cancel").unwrap_or(id);
    let Some(job) = app.jobs().get(job_id) else {
        return err(404, ErrorBody::new("not_found", "no job with this id"));
    };
    if job.tenant_id != principal.tenant_id() {
        // Another tenant's job is a 404, not a 403: the id must not confirm
        // that it exists.
        return err(404, ErrorBody::new("not_found", "no job with this id"));
    }
    if job.state.terminal() {
        // The same answer the console's control plane gives: cancelling
        // something already finished is a state conflict, so a retrying
        // client can tell "not mine" from "already over".
        return err(
            409,
            ErrorBody::new(
                "job_not_cancellable",
                "the job already reached a terminal state",
            ),
        );
    }
    match app.jobs().cancel(job_id) {
        Some(job) => (
            202,
            serde_json::to_string(&job_json(&job, app)).unwrap_or_default(),
        ),
        None => err(404, ErrorBody::new("not_found", "no job with this id")),
    }
}

fn job_json(job: &super::jobs::JobRecord, app: &App) -> JobResponse {
    JobResponse {
        schema: RESPONSE_SCHEMA,
        job_id: job.id.clone(),
        status: job.state.as_str().to_string(),
        url: job.request.url.clone(),
        request_id: job.request_id.clone(),
        scrape: job.response.clone(),
        records: job.records.clone(),
        artifacts: job.artifacts.clone(),
        error: job.error.clone(),
        created_at_ms: job.created_at_ms,
        started_at_ms: job.started_at_ms,
        finished_at_ms: job.finished_at_ms,
        attempts: job.attempts,
        cancelled: job.cancelled,
        chain: app.ledger_head(),
    }
}

// ------------------------------------------------------------ /v1/usage

async fn read_usage(app: &App, head: &Head) -> (u16, String) {
    let principal = match authenticate_key(app, head) {
        Ok(p) => p,
        Err(r) => return r,
    };
    if let Err(e) = principal.require(Scope::UsageRead) {
        return from_api_error(e);
    }
    usage_report(
        app,
        head,
        Some(principal.tenant_id()),
        Some(principal.key_id()),
    )
}

fn usage_report(
    app: &App,
    head: &Head,
    tenant_id: Option<&str>,
    key_id: Option<&str>,
) -> (u16, String) {
    let entries = match app.ledger().read_all() {
        Ok(e) => e,
        Err(e) => return err(500, ErrorBody::new("ledger_unreadable", format!("{e}"))),
    };
    let filter = usage_filter(head, tenant_id, key_id);
    let report = summarize(&entries, &filter);
    let mut out = serde_json::to_value(report_json(&report)).unwrap_or(Value::Null);
    if let Some(o) = out.as_object_mut() {
        o.insert("schema".into(), json!(RESPONSE_SCHEMA));
        // So a caller can confirm the report is consistent with the ledger
        // it was read from.
        o.insert(
            "ledger_head".into(),
            serde_json::to_value(app.ledger_head()).unwrap_or(Value::Null),
        );
    }
    (200, out.to_string())
}

// ----------------------------------------------------------- /v1/admin/*

async fn admin_dispatch(app: &App, head: &Head) -> (u16, String) {
    let method = head.method.as_str();
    let rest = head.rest();
    match (method, rest) {
        ("POST", "tenants") => admin_create_tenant(app, head).await,
        ("GET", "tenants") => admin_list_tenants(app).await,
        ("POST", "keys") => admin_create_key(app, head).await,
        ("GET", "keys") => admin_list_keys(app, head).await,
        ("POST", key) if key.ends_with("/rotate") => admin_rotate_key(app, head, key).await,
        ("POST", key) if key.ends_with("/revoke") => admin_revoke_key(app, head, key).await,
        ("POST", key) if key.ends_with("/suspend") => admin_suspend_key(app, head, key).await,
        ("GET", "usage") => usage_report(app, head, None, query_value(head, "key_id").as_deref()),
        ("POST", "usage/export") => admin_usage_export(app, head).await,
        _ => err(
            404,
            ErrorBody::new("not_found", format!("no admin route for {}", head.path)),
        ),
    }
}

async fn admin_create_tenant(app: &App, head: &Head) -> (u16, String) {
    let body: Value = match parse_json(head) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(name) = body.get("name").and_then(|v| v.as_str()) else {
        return err(400, ErrorBody::new("invalid_request", "name is required"));
    };
    match app.store().new_tenant(name) {
        Ok(tenant) => (
            201,
            serde_json::to_string(&json!({"schema": RESPONSE_SCHEMA, "tenant": tenant}))
                .unwrap_or_default(),
        ),
        Err(e) => err(400, ErrorBody::new("tenant_invalid", format!("{e}"))),
    }
}

async fn admin_list_tenants(app: &App) -> (u16, String) {
    let (tenants, keys) = app.store().stats();
    (
        200,
        serde_json::to_string(&json!({
            "schema": RESPONSE_SCHEMA,
            "tenant_count": tenants,
            "key_count": keys,
            "tenants": app.store().list_tenants(),
        }))
        .unwrap_or_default(),
    )
}

async fn admin_create_key(app: &App, head: &Head) -> (u16, String) {
    let body: Value = match parse_json(head) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let Some(tenant_id) = body.get("tenant_id").and_then(|v| v.as_str()) else {
        return err(
            400,
            ErrorBody::new("invalid_request", "tenant_id is required"),
        );
    };
    let scopes: ScopeSet = match body.get("scopes") {
        Some(v) => {
            let raw: Vec<String> = v
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            match ScopeSet::parse_list(&raw) {
                Ok(s) => s,
                Err(e) => return err(400, ErrorBody::new("invalid_scope", e)),
            }
        }
        None => ScopeSet::parse_list(&["scrape", "jobs_read", "usage_read"].map(str::to_string))
            .unwrap_or_else(|_| ScopeSet::default()),
    };
    let limits = body
        .get("limits")
        .and_then(|v| serde_json::from_value::<crate::tenancy::limits::KeyLimits>(v.clone()).ok())
        .unwrap_or_else(|| app.managed().default_limits.clone());
    if let Err(e) = limits.validate() {
        return err(400, ErrorBody::new("invalid_limits", e));
    }
    if app.store().list_keys(Some(tenant_id)).len() >= app.managed().max_keys_per_tenant {
        return err(
            409,
            ErrorBody::new("too_many_keys", "this tenant has reached its key limit"),
        );
    }
    match app.store().create_key(tenant_id, scopes, limits, None) {
        Ok(generated) => (
            201,
            serde_json::to_string(&json!({
                "schema": RESPONSE_SCHEMA,
                "api_key": generated.plaintext,
                "key": generated.key,
            }))
            .unwrap_or_default(),
        ),
        Err(e) => err(400, ErrorBody::new("key_invalid", format!("{e}"))),
    }
}

async fn admin_list_keys(app: &App, head: &Head) -> (u16, String) {
    let Some(tenant_id) = query_value(head, "tenant_id") else {
        return err(
            400,
            ErrorBody::new("invalid_request", "tenant_id is required"),
        );
    };
    (
        200,
        serde_json::to_string(&json!({
            "schema": RESPONSE_SCHEMA,
            "keys": app.store().list_keys(Some(&tenant_id)),
        }))
        .unwrap_or_default(),
    )
}

async fn admin_rotate_key(app: &App, head: &Head, key: &str) -> (u16, String) {
    if let Err(r) = require_empty_body(head) {
        return r;
    }
    let id = key.strip_suffix("/rotate").unwrap_or(key);
    match app.store().rotate_key(id) {
        Ok(generated) => (
            200,
            serde_json::to_string(&json!({
                "schema": RESPONSE_SCHEMA,
                "api_key": generated.plaintext,
                "key": generated.key,
            }))
            .unwrap_or_default(),
        ),
        Err(e) => err(404, ErrorBody::new("not_found", format!("{e}"))),
    }
}

async fn admin_revoke_key(app: &App, head: &Head, key: &str) -> (u16, String) {
    if let Err(r) = require_empty_body(head) {
        return r;
    }
    let id = key.strip_suffix("/revoke").unwrap_or(key);
    match app.store().revoke_key(id) {
        Ok(record) => (
            200,
            serde_json::to_string(&json!({"schema": RESPONSE_SCHEMA, "key": record}))
                .unwrap_or_default(),
        ),
        Err(e) => err(404, ErrorBody::new("not_found", format!("{e}"))),
    }
}

async fn admin_suspend_key(app: &App, head: &Head, key: &str) -> (u16, String) {
    if let Err(r) = require_empty_body(head) {
        return r;
    }
    let id = key.strip_suffix("/suspend").unwrap_or(key);
    match app.store().suspend_key(id) {
        Ok(record) => (
            200,
            serde_json::to_string(&json!({"schema": RESPONSE_SCHEMA, "key": record}))
                .unwrap_or_default(),
        ),
        Err(e) => err(404, ErrorBody::new("not_found", format!("{e}"))),
    }
}

async fn admin_usage_export(app: &App, head: &Head) -> (u16, String) {
    let entries = match app.ledger().read_all() {
        Ok(e) => e,
        Err(e) => return err(500, ErrorBody::new("ledger_unreadable", format!("{e}"))),
    };
    let filter = usage_filter(head, None, query_value(head, "key_id").as_deref());
    let format = query_value(head, "format").unwrap_or_else(|| "jsonl".to_string());
    let dir = app.managed().export_dir();
    let name = format!(
        "usage-{}",
        crate::usage::ledger::month_of(crate::scrape_api::time_now_ms())
    );
    let out = match format.as_str() {
        "csv" => export_csv(&entries, &filter, dir, &name),
        _ => export_jsonl(&entries, &filter, dir, &name),
    };
    match out {
        Ok(result) => (
            200,
            serde_json::to_string(&json!({
                "schema": RESPONSE_SCHEMA,
                "path": result.path.to_string_lossy(),
                "manifest": result.manifest_path.to_string_lossy(),
                "rows": result.rows,
                "bytes": result.bytes,
                "sha256": result.sha256,
                "truncated": result.truncated,
            }))
            .unwrap_or_default(),
        ),
        Err(e) => err(500, ErrorBody::new("export_failed", format!("{e}"))),
    }
}

// ---------------------------------------------------------------- helpers

/// A key mutation carries no body: the identifier is in the path. A body
/// here is a mistake, not a payload.
fn require_empty_body(head: &Head) -> Result<(), (u16, String)> {
    if head.body.is_empty() {
        Ok(())
    } else {
        Err(err(
            400,
            ErrorBody::new("invalid_request", "this operation takes no request body"),
        ))
    }
}

fn parse_json(head: &Head) -> Result<Value, (u16, String)> {
    if head.body.is_empty() {
        return Ok(Value::Object(Default::default()));
    }
    serde_json::from_slice(&head.body)
        .map_err(|e| err(400, ErrorBody::new("invalid_json", e.to_string())))
}

fn parse_body<T: serde::de::DeserializeOwned>(
    head: &Head,
    cfg: &crate::config::ManagedApiCfg,
) -> Result<T, (u16, String)> {
    if head.body.len() > cfg.max_request_bytes as usize {
        return Err(err(
            413,
            ErrorBody::new(
                "request_too_large",
                "the request body exceeds the configured limit",
            ),
        ));
    }
    let value = parse_json(head)?;
    serde_json::from_value::<T>(value)
        .map_err(|e| err(400, ErrorBody::new("invalid_request", e.to_string())))
}

fn query_value(head: &Head, key: &str) -> Option<String> {
    for pair in head.query.split('&') {
        let (k, v) = match pair.split_once('=') {
            Some((k, v)) => (k, v),
            None => (pair, ""),
        };
        if k == key {
            return Some(urldecode(v));
        }
    }
    None
}

fn query_usize(head: &Head, key: &str) -> Option<usize> {
    query_value(head, key).and_then(|v| v.parse().ok())
}

fn urldecode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn usage_filter(head: &Head, tenant_id: Option<&str>, key_id: Option<&str>) -> UsageFilter {
    UsageFilter {
        tenant_id: tenant_id.map(|t| t.to_string()),
        key_id: key_id.map(|k| k.to_string()),
        since_ms: query_usize(head, "since").map(|v| v as u64),
        until_ms: query_usize(head, "until").map(|v| v as u64),
        limit: query_usize(head, "limit").unwrap_or(crate::config::V1_USAGE_QUERY_MAX_ROWS),
    }
}

fn err(status: u16, body: ErrorBody) -> (u16, String) {
    (status, err_json(body))
}

fn err_json(body: ErrorBody) -> String {
    serde_json::to_string(&ErrorResponse::new(body)).unwrap_or_else(|_| "{}".into())
}

/// Meter one request into the append-only usage ledger.
///
/// The write never changes the response, but it is never skipped either: a
/// ledger that quietly stopped advancing would make every later report lie,
/// so a failure is logged at the level an operator acts on.
#[allow(clippy::too_many_arguments)]
fn meter(
    app: &App,
    tenant_id: &str,
    key_id: &str,
    request_id: &str,
    units: u32,
    render: RenderMode,
    screenshot: bool,
    proxy_region: Option<String>,
    status: &str,
    status_code: u16,
    bytes: u64,
    duration_ms: u64,
) {
    if let Err(e) = app.ledger().record(
        tenant_id,
        key_id,
        request_id,
        units,
        render,
        screenshot,
        proxy_region,
        status,
        status_code,
        bytes,
        duration_ms,
    ) {
        tracing::error!(
            tenant = %tenant_id,
            key = %key_id,
            request_id = %request_id,
            "cannot append to the usage ledger: {e}"
        );
    }
}

/// The ledger's outcome class: `ok`, `refused` (policy, SSRF or a limit the
/// key tripped) or `error` (anything else that went wrong).
fn outcome_class(status_code: u16) -> &'static str {
    match status_code {
        200 => "ok",
        403 | 429 => "refused",
        _ => "error",
    }
}

fn metered_error(e: crate::ratelimit::RateLimitError) -> ErrorBody {
    use crate::ratelimit::RateLimitError;
    match e {
        RateLimitError::RateLimited { retry_after_secs } => {
            ErrorBody::new("rate_limited", "too many requests for this key")
                .with_retry_after(retry_after_secs * 1_000)
        }
        RateLimitError::ConcurrencyLimited { retry_after_secs } => ErrorBody::new(
            "concurrency_limited",
            "too many concurrent requests for this key",
        )
        .with_retry_after(retry_after_secs * 1_000),
        RateLimitError::QuotaExhausted { reset_at } => {
            ErrorBody::new("quota_exhausted", "monthly quota exhausted")
                .with_retry_after(reset_at.max(0) as u64 * 1_000)
        }
    }
}

/// Deliver the finished-scrape webhook.
async fn deliver_webhook(
    app: &App,
    tenant_id: &str,
    key_id: &str,
    url: &str,
    response: &ScrapeResponse,
    delivery_id: &str,
) {
    if !app.managed().webhook.enabled {
        return;
    }
    let payload = json!({
        "schema": RESPONSE_SCHEMA,
        "event": "scrape.completed",
        "tenant_id": tenant_id,
        "key_id": key_id,
        "request_id": response.request_id,
        "status": response.status,
        "url": response.url,
        "final_url": response.final_url,
        "bytes": response.bytes,
        "usage_units": response.usage_units,
        "cost": response.cost,
        "scrape": response,
    });
    let delivery =
        super::webhook::Delivery::new(delivery_id, tenant_id, key_id, url, "scrape.completed");
    app.webhooks().deliver(&delivery, &payload).await;
}

use super::idempotency::Insert;
