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
            .redirect(reqwest::redirect::Policy::limited(5));
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
        metrics.robot_fetched();
        match self.try_once(&robots, metrics, None).await {
            Ok(o) if o.status == 200 && o.content_type.starts_with("text") => Some(o.body),
            _ => None,
        }
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
        let mut rb = client.get(url.as_str());
        if let Some(hdrs) = extra {
            for (k, v) in hdrs {
                rb = rb.header(k.as_str(), v.as_str());
            }
        }
        let mut resp = match rb.send().await {
            Ok(r) => r,
            Err(e) => {
                let latency = t0.elapsed().as_millis() as u64;
                return (
                    Err(FetchError::Failed(e.to_string())),
                    AttemptTrace {
                        transport_failed: true,
                        latency_ms: latency,
                        ..trace0
                    },
                );
            }
        };
        let status = resp.status().as_u16();
        let ct = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();
        let retry_after = parse_retry_after(resp.headers());
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
                        Err(FetchError::Failed(format!("body read: {e}"))),
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
            url: url.clone(),
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
            let (res, trace) = self.try_once_traced(&plan.url, metrics, headers).await;
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
                                tokio::time::sleep(delay).await;
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
                tokio::time::sleep(Duration::from_millis(wait)).await;
            }
        }
    }
}

fn is_block(status: u16) -> bool {
    matches!(status, 401 | 403 | 429 | 451)
}

/// Parse Retry-After: delay-seconds or HTTP date (best effort).
pub fn parse_retry_after(h: &reqwest::header::HeaderMap) -> Option<Duration> {
    let v = h.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    if let Ok(secs) = v.trim().parse::<u64>() {
        return Some(Duration::from_secs(secs.min(3600)));
    }
    // HTTP-date: compute delta from now; unparsable dates => None.
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    // Minimal IMF-fixdate parse: "Wed, 21 Oct 2015 07:28:00 GMT"
    let parts: Vec<&str> = v.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = months
        .iter()
        .position(|m| m.eq_ignore_ascii_case(parts[2]))? as u64;
    let year: u64 = parts[3].parse().ok()?;
    let day: u64 = parts[1].parse().ok()?;
    let hhmm: Vec<&str> = parts[4].split(':').collect();
    if hhmm.len() != 3 {
        return None;
    }
    let (h, m, s) = (
        hhmm[0].parse::<u64>().ok()?,
        hhmm[1].parse::<u64>().ok()?,
        hhmm[2].parse::<u64>().ok()?,
    );
    // Days since epoch, civil-from-days algorithm inverted (days since 1970):
    let y = if mon < 2 { year + 4800 } else { year + 4799 };
    let m2 = (mon + 1) % 12;
    let days = day + (153 * m2 + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32082 - 719468;
    let when = days * 86400 + h * 3600 + m * 60 + s;
    if when > now {
        Some(Duration::from_secs((when - now).min(3600)))
    } else {
        Some(Duration::ZERO)
    }
}

#[cfg(test)]
mod tests;
