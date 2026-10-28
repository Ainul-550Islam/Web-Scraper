//! HTTP transport: one `reqwest::Client` per egress (direct or proxy),
//! honest user-agent, timeouts, retry/backoff with jitter, and strict
//! classification of "the server said no" responses.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::json;
use url::Url;

use crate::config::Config;
use crate::events::EventKind;
use crate::exec::{ExecutionMeta, RequestContext, RequestPlan};
use crate::metrics::Metrics;
use crate::proxy::ProxyPool;
use crate::retry::{RetryDecision, RetryPolicy};

#[derive(Debug, Clone)]
pub enum FetchError {
    /// 401/403/429/451 — access refusal. The engine records a block and the
    /// politeness layer backs off the whole domain. Never retried away.
    Blocked {
        status: u16,
        retry_after: Option<Duration>,
    },
    /// robots.txt says no.
    RobotsDenied,
    /// Transport/5xx failures after exhausting retries.
    Failed(String),
    /// A local safety limit rejected the exchange (response larger than
    /// `max_response_bytes`, proxy URL the builder refuses). Retrying cannot
    /// change the answer, so the retry loop short-circuits on this.
    Permanent(String),
}

/// Text form used in DLQ reasons, events and job failure records.
impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Blocked {
                status,
                retry_after,
            } => match retry_after {
                Some(d) => write!(f, "blocked: status {status}, retry-after {d:?}"),
                None => write!(f, "blocked: status {status}"),
            },
            FetchError::RobotsDenied => write!(f, "denied by robots.txt"),
            FetchError::Failed(s) => write!(f, "{s}"),
            FetchError::Permanent(s) => write!(f, "permanent: {s}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FetchOutcome {
    pub url: Url,
    /// Validators for incremental revalidation (`Some` only when the server
    /// actually sent them).
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub status: u16,
    pub content_type: String,
    pub body: String,
    pub latency_ms: u64,
    pub proxy_id: String,
    pub bytes: usize,
}

pub struct Fetcher {
    cfg: Arc<Config>,
    pool: Arc<ProxyPool>,
    clients: RwLock<HashMap<String, reqwest::Client>>,
    salt: AtomicCounter,
    /// Retry math and policy, factored into `retry` so unit tests (and the
    /// engine's retry-stats reporting) run against the same object the
    /// live loop uses. The legacy `get`/`get_with` path and `execute` share
    /// it, so there is exactly one delay curve in the binary.
    pub policy: RetryPolicy,
    stops: Vec<Arc<std::sync::atomic::AtomicBool>>,
    pacing: Option<Arc<crate::policy::Politeness>>,
    #[cfg(feature = "redis")]
    shared_pacing: Option<crate::dist::Queue>,
    robots: RwLock<HashMap<String, Arc<crate::robots::Robots>>>,
}

#[derive(Default)]
struct AtomicCounter(std::sync::atomic::AtomicU64);

impl AtomicCounter {
    fn next(&self) -> u64 {
        let base = self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        base.wrapping_mul(2654435761)
            .wrapping_add(nanos)
            .wrapping_add(base << 7)
    }
}

impl Fetcher {
    pub fn new(cfg: Arc<Config>, pool: Arc<ProxyPool>) -> Self {
        let policy = RetryPolicy::from_cfg(&cfg);
        Self {
            cfg,
            pool,
            clients: RwLock::new(HashMap::new()),
            salt: AtomicCounter::default(),
            policy,
            stops: Vec::new(),
            pacing: None,
            #[cfg(feature = "redis")]
            shared_pacing: None,
            robots: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn with_cancellation(
        mut self,
        stops: Vec<Arc<std::sync::atomic::AtomicBool>>,
        pacing: Option<Arc<crate::policy::Politeness>>,
    ) -> Self {
        self.stops = stops;
        self.pacing = pacing;
        self
    }
    #[cfg(feature = "redis")]
    pub(crate) fn with_shared_pacing(mut self, queue: crate::dist::Queue) -> Self {
        self.shared_pacing = Some(queue);
        self
    }
    #[cfg(feature = "redis")]
    pub(crate) fn remember_robots(&self, base: &Url, text: Option<&str>) {
        let rules = text.map_or_else(crate::robots::Robots::default, |s| {
            crate::robots::Robots::parse(s, &crate::engine::ua_token(&self.cfg.user_agent))
        });
        self.robots
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(base.origin().ascii_serialization(), Arc::new(rules));
    }
    pub(crate) async fn wait_initial(&self, host: &str, metrics: &Metrics) -> bool {
        if let Some(pacing) = &self.pacing {
            if pacing.is_abandoned(host, &self.cfg) {
                return false;
            }
            let wait = pacing.wait(host, &self.cfg, metrics);
            tokio::pin!(wait);
            loop {
                if self.stopped() {
                    return false;
                }
                tokio::select! { _ = &mut wait => return !self.stopped(), _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
            }
        }
        !self.stopped()
    }
    fn stopped(&self) -> bool {
        self.stops
            .iter()
            .any(|s| s.load(std::sync::atomic::Ordering::Relaxed))
    }
    async fn pause(&self, delay: Duration) -> bool {
        let mut sleep = Box::pin(tokio::time::sleep(delay));
        loop {
            if self.stopped() {
                return false;
            }
            tokio::select! { _ = &mut sleep => return true, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
        }
    }

    fn client_for(&self, proxy_idx: usize) -> Result<reqwest::Client> {
        let id = self.pool.id(proxy_idx);
        if let Some(c) = self
            .clients
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(&id)
        {
            return Ok(c.clone());
        }
        let url = self.pool.proxy_url(proxy_idx).flatten();
        let mut b = reqwest::Client::builder()
            .user_agent(&self.cfg.user_agent)
            .timeout(Duration::from_secs(self.cfg.request_timeout_secs))
            .connect_timeout(Duration::from_secs(self.cfg.connect_timeout_secs))
            .redirect(reqwest::redirect::Policy::none());
        // No cookie store: the `cookies` feature is intentionally not enabled,
        // so each request is stateless per egress (explicit isolation, not spoofing).
        if let Some(u) = url {
            // The proxy URL may carry `user:password@` — never put the raw
            // string into an error that can end up in a log line.
            b = b.proxy(reqwest::Proxy::all(&u).with_context(|| {
                format!(
                    "proxy {}: bad url {}",
                    id,
                    crate::config::redact_credentials(&u)
                )
            })?);
        }
        let c = b.build().context("failed to build http client")?;
        self.clients
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), c.clone());
        Ok(c)
    }

    pub async fn get(&self, url: Url, metrics: &Metrics) -> Result<FetchOutcome, FetchError> {
        get_with_retries(self, url, metrics, None).await
    }

    /// Conditional GET: extra request headers (If-None-Match /
    /// If-Modified-Since). A 304 comes back as `Ok` with an empty body —
    /// "not modified" is an answer, not an error, and must not be retried.
    pub async fn get_with(
        &self,
        url: Url,
        metrics: &Metrics,
        headers: Vec<(String, String)>,
    ) -> Result<FetchOutcome, FetchError> {
        get_with_retries(
            self,
            url,
            metrics,
            if headers.is_empty() {
                None
            } else {
                Some(&headers)
            },
        )
        .await
    }

    /// robots.txt for `base` (scheme://authority). None = not found/unreadable
    /// (treat as allow-all per common crawler practice, still logged).
    pub async fn fetch_robots(&self, base: &Url, metrics: &Metrics) -> Option<String> {
        let mut robots = base.clone();
        robots.set_path("/robots.txt");
        robots.set_query(None);
        robots.set_fragment(None);
        if !self
            .wait_initial(base.host_str().unwrap_or_default(), metrics)
            .await
        {
            return Some(crate::robots::UNAVAILABLE_POLICY.into());
        }
        metrics.robot_fetched();
        metrics.request_started();
        let text = match self
            .try_once_scoped(&robots, metrics, None, None, true)
            .await
            .0
        {
            Ok(o)
                if o.status == 200
                    && (o.content_type.starts_with("text/plain")
                        || o.body.to_ascii_lowercase().contains("user-agent:")) =>
            {
                Some(o.body)
            }
            Ok(o) if matches!(o.status, 404 | 410) => None,
            Err(FetchError::Blocked { retry_after, .. }) => {
                metrics.request_blocked();
                if let Some(pacing) = &self.pacing {
                    pacing.note_block(base.host_str().unwrap_or_default(), &self.cfg, retry_after);
                }
                #[cfg(feature = "redis")]
                if let Some(queue) = &self.shared_pacing {
                    let _ = queue
                        .clone()
                        .note_block(base.host_str().unwrap_or_default(), &self.cfg, retry_after)
                        .await;
                }
                Some(crate::robots::UNAVAILABLE_POLICY.into())
            }
            _ => {
                tracing::warn!(origin = %crate::config::redact_url_origin(base.as_str()), "robots.txt unavailable/refused; denying this origin until cache refresh");
                Some(crate::robots::UNAVAILABLE_POLICY.to_string())
            }
        };
        let policy = Arc::new(
            text.as_ref()
                .map(|t| {
                    crate::robots::Robots::parse(t, &crate::engine::ua_token(&self.cfg.user_agent))
                })
                .unwrap_or_default(),
        );
        if let Some(delay) = policy.required_delay() {
            if let Some(pacing) = &self.pacing {
                pacing.note_crawl_delay(base.host_str().unwrap_or_default(), &self.cfg, delay);
            }
            #[cfg(feature = "redis")]
            if let Some(queue) = &self.shared_pacing {
                let _ = queue
                    .clone()
                    .note_crawl_delay(base.host_str().unwrap_or_default(), delay)
                    .await;
            }
        }
        self.robots
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(base.origin().ascii_serialization(), policy);
        text
    }

    async fn try_once(
        &self,
        url: &Url,
        metrics: &Metrics,
        extra: Option<&Vec<(String, String)>>,
    ) -> Result<FetchOutcome, FetchError> {
        self.try_once_traced(url, metrics, extra).await.0
    }

    /// Same exchange, plus the egress attribution the event layer needs
    /// (which proxy, whether transport died). The extra struct is free for
    /// the legacy path (single extra move).
    #[allow(clippy::type_complexity)]
    async fn try_once_traced(
        &self,
        url: &Url,
        metrics: &Metrics,
        extra: Option<&Vec<(String, String)>>,
    ) -> (Result<FetchOutcome, FetchError>, AttemptTrace) {
        self.try_once_scoped(url, metrics, extra, None, false).await
    }

    #[allow(clippy::type_complexity)]
    async fn try_once_scoped(
        &self,
        url: &Url,
        metrics: &Metrics,
        extra: Option<&Vec<(String, String)>>,
        target: Option<&crate::config::TargetCfg>,
        robots_request: bool,
    ) -> (Result<FetchOutcome, FetchError>, AttemptTrace) {
        let request = self.try_once_scoped_inner(url, metrics, extra, target, robots_request);
        tokio::pin!(request);
        loop {
            if self.stopped() {
                return (
                    Err(FetchError::Permanent("request cancelled".into())),
                    AttemptTrace {
                        proxy_id: "cancelled".into(),
                        transport_failed: false,
                        latency_ms: 0,
                    },
                );
            }
            tokio::select! { out = &mut request => return out, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
        }
    }
    #[allow(clippy::type_complexity)]
    async fn try_once_scoped_inner(
        &self,
        url: &Url,
        metrics: &Metrics,
        extra: Option<&Vec<(String, String)>>,
        target: Option<&crate::config::TargetCfg>,
        robots_request: bool,
    ) -> (Result<FetchOutcome, FetchError>, AttemptTrace) {
        let idx = self.pool.pick(self.salt.next());
        let trace0 = AttemptTrace {
            proxy_id: self.pool.id(idx),
            transport_failed: false,
            latency_ms: 0,
        };
        // In-flight budget: with `max_inflight_per_proxy` set, an attempt
        // that would exceed the cap fails as a retryable transport refusal
        // (backoff handles the wait) instead of piling requests onto one
        // egress. Unlimited by default — the guard costs one counter pair.
        let _hold = match self.pool.clone().try_hold(idx) {
            Some(h) => h,
            None => {
                metrics.note_budget_denied(crate::metrics::BudgetScope::Proxy);
                return (
                    Err(FetchError::Failed(format!(
                        "proxy `{}` is at its in-flight cap",
                        self.pool.id(idx)
                    ))),
                    trace0,
                );
            }
        };
        let client = match self.client_for(idx) {
            Ok(c) => c,
            Err(e) => return (Err(FetchError::Failed(e.to_string())), trace0),
        };
        let t0 = Instant::now();
        let mut current = url.clone();
        let mut redirects = 0;
        let mut resp = loop {
            if self.stopped() {
                return (
                    Err(FetchError::Permanent("request cancelled".into())),
                    trace0,
                );
            }
            #[cfg(feature = "redis")]
            if let Some(shared) = &self.shared_pacing {
                let mut queue = shared.clone();
                let host = current.host_str().unwrap_or_default();
                if let Err(e) = queue
                    .wait_policy(host, &self.cfg.policy_for(host), self.stops.first())
                    .await
                {
                    return (Err(FetchError::Permanent(e.to_string())), trace0);
                }
                if self.stopped() {
                    return (
                        Err(FetchError::Permanent("request cancelled".into())),
                        trace0,
                    );
                }
            }
            let mut rb = client.get(current.as_str());
            // Validators belong to the requested resource, not to a redirect destination.
            if redirects == 0 {
                if let Some(hdrs) = extra {
                    for (k, v) in hdrs {
                        rb = rb.header(k.as_str(), v.as_str());
                    }
                }
            }
            let response = match rb.send().await {
                Ok(r) => r,
                Err(e) => {
                    return (
                        Err(FetchError::Failed(e.without_url().to_string())),
                        AttemptTrace {
                            transport_failed: true,
                            latency_ms: t0.elapsed().as_millis() as u64,
                            ..trace0
                        },
                    )
                }
            };
            if !response.status().is_redirection() || response.status().as_u16() == 304 {
                break response;
            }
            if redirects >= 5 {
                return (
                    Err(FetchError::Permanent(
                        "redirect limit exceeded (5 hops)".into(),
                    )),
                    trace0,
                );
            }
            let next = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|h| h.to_str().ok())
                .and_then(|h| current.join(h).ok());
            let Some(next) = next else {
                return (
                    Err(FetchError::Permanent(
                        "redirect has no valid Location".into(),
                    )),
                    trace0,
                );
            };
            if next.origin() != url.origin()
                || next.username() != url.username()
                || next.password() != url.password()
            {
                return (
                    Err(FetchError::Permanent(
                        "redirect destination is outside the approved origin".into(),
                    )),
                    trace0,
                );
            }
            if !robots_request {
                if target.is_some_and(|t| {
                    (!t.include.is_empty() && !t.include.iter().any(|p| next.as_str().contains(p)))
                        || t.exclude.iter().any(|p| next.as_str().contains(p))
                }) {
                    return (
                        Err(FetchError::Permanent(
                            "redirect destination is outside target include/exclude scope".into(),
                        )),
                        trace0,
                    );
                }
                if self.cfg.respect_robots {
                    let rules = self
                        .robots
                        .read()
                        .unwrap_or_else(|p| p.into_inner())
                        .get(&url.origin().ascii_serialization())
                        .cloned();
                    if rules.is_none_or(|r| !r.allows_url(&next)) {
                        metrics.robot_denied();
                        return (Err(FetchError::RobotsDenied), trace0);
                    }
                }
            }
            let redirect_wait = parse_retry_after(response.headers());
            drop(response);
            if let Some(delay) = redirect_wait {
                if delay > Duration::from_secs(3600) {
                    return (Err(FetchError::Permanent("redirect Retry-After exceeds the supported wait; destination was not fetched".into())), trace0);
                }
                if !self.pause(delay).await {
                    return (
                        Err(FetchError::Permanent("request cancelled".into())),
                        trace0,
                    );
                }
            }
            if let Some(pacing) = &self.pacing {
                let mut permit =
                    Box::pin(pacing.wait(next.host_str().unwrap_or_default(), &self.cfg, metrics));
                loop {
                    if self.stopped() {
                        return (
                            Err(FetchError::Permanent("request cancelled".into())),
                            trace0,
                        );
                    }
                    tokio::select! { _ = &mut permit => break, _ = tokio::time::sleep(Duration::from_millis(100)) => {} }
                }
            }
            metrics.request_started();
            redirects += 1;
            current = next;
        };
        let status = resp.status().as_u16();
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let retry_after = parse_retry_after(resp.headers());
        if matches!(status, 401 | 403 | 429 | 451) || (status == 503 && retry_after.is_some()) {
            self.pool.report(idx, true, t0.elapsed().as_millis() as u64);
            return (
                Err(FetchError::Blocked {
                    status,
                    retry_after,
                }),
                trace0,
            );
        }
        if !robots_request && (400..500).contains(&status) {
            self.pool.report(idx, true, t0.elapsed().as_millis() as u64);
            return (
                Err(FetchError::Permanent(format!("upstream HTTP {status}"))),
                trace0,
            );
        }
        let etag = resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let last_modified = resp
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        // Bounded body: pre-check the declared length, then cap the stream —
        // a lying or absent content-length must not turn into unbounded
        // buffering. Reading via chunks (instead of `.text()`) decodes as
        // UTF-8-with-replacement; hostile non-UTF-8 pages degrade to U+FFFD,
        // never to a panic.
        let cap = (self.cfg.max_response_bytes).max(1024) as usize;
        if let Some(declared) = resp.content_length() {
            if declared > cap as u64 {
                let latency = t0.elapsed().as_millis() as u64;
                self.pool.report(idx, true, latency);
                return (
                    Err(FetchError::Permanent(format!(
                        "declared body size {declared} bytes exceeds max_response_bytes {cap}"
                    ))),
                    AttemptTrace {
                        latency_ms: latency,
                        ..trace0
                    },
                );
            }
        }
        let mut acc: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    if acc.len() + chunk.len() > cap {
                        let latency = t0.elapsed().as_millis() as u64;
                        self.pool.report(idx, true, latency);
                        return (
                            Err(FetchError::Permanent(format!(
                                "streamed body exceeds max_response_bytes {cap}"
                            ))),
                            AttemptTrace {
                                latency_ms: latency,
                                ..trace0
                            },
                        );
                    }
                    acc.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    let latency = t0.elapsed().as_millis() as u64;
                    self.pool.report(idx, false, latency);
                    return (
                        Err(FetchError::Failed(format!(
                            "body read: {}",
                            e.without_url()
                        ))),
                        AttemptTrace {
                            transport_failed: true,
                            latency_ms: latency,
                            ..trace0
                        },
                    );
                }
            }
        }
        let body = String::from_utf8_lossy(&acc).into_owned();
        let latency = t0.elapsed().as_millis() as u64;
        let outcome = FetchOutcome {
            url: current,
            etag,
            last_modified,
            status,
            content_type: ct,
            bytes: body.len(),
            body,
            latency_ms: latency,
            proxy_id: self.pool.id(idx),
        };
        // Classification: refusals are not "failures" to hammer — they are
        // answers. Blocks are reported to the pool as *neutral* (proxy worked;
        // the site refused), transport errors drain the proxy.
        self.pool.report(idx, true, latency);
        metrics.request_ok();
        if is_block(status) {
            return (
                Err(FetchError::Blocked {
                    status,
                    retry_after,
                }),
                AttemptTrace {
                    latency_ms: latency,
                    ..trace0
                },
            );
        }
        if status >= 500 {
            return (
                Err(FetchError::Failed(format!("server status {status}"))),
                AttemptTrace {
                    latency_ms: latency,
                    ..trace0
                },
            );
        }
        (
            Ok(outcome),
            AttemptTrace {
                latency_ms: latency,
                ..trace0
            },
        )
    }

    /// Engine-path fetch: identical transport rules and retry formula as
    /// the legacy loop, plus plan deadline enforcement, the shared retry
    /// budget, structured event publication and execution metadata. Called
    /// by crawl workers; `get`/`get_with` stay as they were for robots,
    /// sitemap and any external caller (no events, no plan, no meta).
    pub async fn execute(
        &self,
        plan: RequestPlan,
        ctx: &RequestContext,
        metrics: &Metrics,
    ) -> Result<(FetchOutcome, ExecutionMeta), (FetchError, ExecutionMeta)> {
        let mut meta = ExecutionMeta {
            correlation_id: ctx.correlation_id,
            ..Default::default()
        };
        let headers = (!plan.headers.is_empty()).then_some(&plan.headers);
        let started = Instant::now();
        let mut attempt = 0u32;
        loop {
            // Deadline first: an attempt that cannot finish in time is not
            // started at all (this is what makes job deadlines real rather
            // than advisory).
            if self.stopped() {
                return Err((FetchError::Permanent("request cancelled".into()), meta));
            }
            if plan.expired(Instant::now()) {
                meta.total_ms = started.elapsed().as_millis() as u64;
                meta.deadline_hit = true;
                return Err((
                    FetchError::Failed(format!(
                        "deadline exceeded for {}",
                        crate::config::redact_credentials(plan.url.as_str())
                    )),
                    meta,
                ));
            }
            attempt += 1;
            meta.attempts = attempt;
            metrics.request_started();
            ctx.publish(
                EventKind::RequestStarted,
                json!({"url": plan.url.as_str(), "attempt": attempt}),
            );
            let (res, trace) = self
                .try_once_scoped(
                    &plan.url,
                    metrics,
                    headers,
                    self.cfg.targets.get(ctx.target),
                    false,
                )
                .await;
            meta.egress = trace.proxy_id.clone();
            meta.last_attempt_ms = trace.latency_ms;
            ctx.publish(
                EventKind::ProxySelected,
                json!({"url": plan.url.as_str(), "egress": trace.proxy_id, "attempt": attempt}),
            );
            if trace.transport_failed {
                ctx.publish(
                    EventKind::ProxyFailure,
                    json!({"url": plan.url.as_str(), "egress": trace.proxy_id, "attempt": attempt}),
                );
            }
            match res {
                Ok(o) => {
                    if o.status != 304 {
                        metrics.page_fetched();
                    }
                    meta.bytes = o.bytes as u64;
                    meta.status = o.status;
                    meta.total_ms = started.elapsed().as_millis() as u64;
                    ctx.acct_note_bytes(o.bytes as u64);
                    ctx.publish(
                        EventKind::RequestFinished,
                        json!({
                            "url": plan.url.as_str(),
                            "status": o.status,
                            "ms": o.latency_ms,
                            "bytes": o.bytes,
                            "attempt": attempt,
                        }),
                    );
                    return Ok((o, meta));
                }
                Err(e) => match e {
                    e @ (FetchError::Blocked { .. }
                    | FetchError::RobotsDenied
                    | FetchError::Permanent(_)) => {
                        meta.total_ms = started.elapsed().as_millis() as u64;
                        let status = match &e {
                            FetchError::Blocked { status, .. } => Some(*status),
                            _ => None,
                        };
                        ctx.publish(
                            EventKind::RequestFinished,
                            json!({
                                "url": plan.url.as_str(),
                                "status": status,
                                "error": e.to_string(),
                                "attempt": attempt,
                            }),
                        );
                        return Err((e, meta));
                    }
                    FetchError::Failed(msg) => {
                        let sample = self.salt.next() % 251;
                        match self
                            .policy
                            .decide(attempt, &msg, false, false, false, sample)
                        {
                            RetryDecision::Retry { delay, reason, .. } => {
                                // Budget gate: under a global limit, a
                                // healthy-looking retry storm still cannot
                                // outrun the run's declared retry appetite.
                                if let Some(b) = &ctx.budget {
                                    if !b.try_consume() {
                                        metrics.retry_budget_denied();
                                        ctx.publish(
                                            EventKind::RetryScheduled,
                                            json!({
                                                "url": plan.url.as_str(),
                                                "granted": false,
                                                "reason": reason.as_str(),
                                            }),
                                        );
                                        meta.total_ms = started.elapsed().as_millis() as u64;
                                        return Err((
                                            FetchError::Failed(format!(
                                                "{msg} (retry budget exhausted)"
                                            )),
                                            meta,
                                        ));
                                    }
                                }
                                // The exchange ends if the wait would eat
                                // the remaining deadline — fail now with a
                                // deadline-shaped error instead.
                                let wait = delay.as_millis() as u64;
                                if plan
                                    .remaining(Instant::now())
                                    .is_some_and(|rem| rem < delay)
                                {
                                    meta.total_ms = started.elapsed().as_millis() as u64;
                                    meta.deadline_hit = true;
                                    return Err((
                                        FetchError::Failed(format!(
                                            "deadline exceeded for {}",
                                            crate::config::redact_credentials(plan.url.as_str())
                                        )),
                                        meta,
                                    ));
                                }
                                metrics.request_retried();
                                metrics.note_backoff_ms(wait);
                                ctx.acct_note_retry(wait);
                                meta.backoff_ms += wait;
                                ctx.publish(
                                    EventKind::RetryScheduled,
                                    json!({
                                        "url": plan.url.as_str(),
                                        "granted": true,
                                        "attempt": attempt + 1,
                                        "delay_ms": wait,
                                        "reason": reason.as_str(),
                                    }),
                                );
                                if !self.pause(delay).await {
                                    return Err((
                                        FetchError::Permanent("request cancelled".into()),
                                        meta,
                                    ));
                                }
                            }
                            RetryDecision::Exhausted { .. } => {
                                meta.total_ms = started.elapsed().as_millis() as u64;
                                return Err((
                                    FetchError::Failed(format!(
                                        "{msg} (after {} retries)",
                                        self.policy.max_retries
                                    )),
                                    meta,
                                ));
                            }
                            RetryDecision::NoRetry => {
                                meta.total_ms = started.elapsed().as_millis() as u64;
                                return Err((FetchError::Failed(msg), meta));
                            }
                        }
                    }
                },
            }
        }
    }
}

/// Egress attribution for one `try_once_traced` attempt (event layer).
#[derive(Debug, Clone)]
pub struct AttemptTrace {
    pub proxy_id: String,
    pub transport_failed: bool,
    pub latency_ms: u64,
}

/// Async retry/backoff loop around `try_once`.
async fn get_with_retries(
    f: &Fetcher,
    url: Url,
    metrics: &Metrics,
    extra: Option<&Vec<(String, String)>>,
) -> Result<FetchOutcome, FetchError> {
    let max_retries = f.cfg.max_retries;
    let base = f.cfg.retry_base_ms;
    let cap = f.cfg.max_single_wait_ms;

    let mut attempt = 0u32;
    loop {
        metrics.request_started();
        match f.try_once(&url, metrics, extra).await {
            Ok(o) => {
                if o.status != 304 {
                    metrics.page_fetched();
                }
                return Ok(o);
            }
            Err(
                e @ (FetchError::Blocked { .. }
                | FetchError::RobotsDenied
                | FetchError::Permanent(_)),
            ) => return Err(e),
            Err(FetchError::Failed(msg)) => {
                attempt += 1;
                if attempt > max_retries {
                    return Err(FetchError::Failed(format!(
                        "{msg} (after {max_retries} retries)"
                    )));
                }
                metrics.request_retried();
                let exp = base.saturating_mul(1u64 << (attempt - 1).min(5));
                let jitter = f.salt.next() % 251;
                let wait = (exp + jitter).min(cap);
                metrics.note_backoff_ms(wait);
                if !f.pause(Duration::from_millis(wait)).await {
                    return Err(FetchError::Permanent("request cancelled".into()));
                }
            }
        }
    }
}

fn is_block(status: u16) -> bool {
    matches!(status, 401 | 403 | 429 | 451)
}

/// Parse Retry-After: delay-seconds or HTTP date (best effort).
pub fn parse_retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> {
    let v = h.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = httpdate::parse_http_date(v).ok()?;
    Some(
        when.duration_since(SystemTime::now())
            .unwrap_or(Duration::ZERO),
    )
}

#[cfg(test)]
mod tests;
