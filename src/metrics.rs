use std::sync::atomic::{AtomicU64, Ordering};

/// The closed label vocabulary for budget denials. Deliberately an enum:
/// the scope strings reaching metrics can never be user input (fixed-label
/// rule for `render_prometheus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetScope {
    Proxy,
    Records,
}

/// Flat, dependency-free counters. Everything a Grafana panel could want for
/// a compliance-first engine: throughput, politeness waits, robots denials,
/// backoff events — not just raw request counts.
#[derive(Debug, Default)]
pub struct Metrics {
    pub requests_total: AtomicU64,
    pub requests_ok: AtomicU64,
    pub requests_retry: AtomicU64,
    pub requests_blocked: AtomicU64,
    pub backoff_sleep_ms_total: AtomicU64,
    pub politeness_sleeps: AtomicU64,
    pub politeness_permits: AtomicU64,
    pub politeness_host_locks: AtomicU64,
    pub robots_fetches: AtomicU64,
    pub checkpoints_written: AtomicU64,
    pub requests_revalidated: AtomicU64,
    pub robots_denials: AtomicU64,
    pub pages_fetched: AtomicU64,
    pub records_extracted: AtomicU64,
    pub records_deduped: AtomicU64,
    pub browser_sessions_created: AtomicU64,
    pub browser_challenges_detected: AtomicU64,
    pub browser_actions_completed: AtomicU64,
    pub selector_records_previewed: AtomicU64,
    pub records_written: AtomicU64,
    pub domains_abandoned: AtomicU64,
    // Engine layer (job lifecycle / scheduler / breaker / budget). These
    // complement the request-level counters above; they count JOBS and
    // queue/policy decisions, which is what operators page over when the
    // request numbers look fine but the crawl is stuck.
    pub jobs_completed: AtomicU64,
    pub jobs_failed: AtomicU64,
    pub queue_rejected: AtomicU64,
    pub breaker_rejected: AtomicU64,
    pub retry_budget_denied: AtomicU64,
    // Resource budgets (see `crate::budget`) — fixed scopes only.
    pub budget_denied_proxy: AtomicU64,
    pub budget_denied_records: AtomicU64,
    // Extraction pipeline (transform/schema layer).
    pub records_quarantined: AtomicU64,
    pub schema_drifts: AtomicU64,
    pub pipeline_failures: AtomicU64,
    // Operational telemetry. These measure the control plane itself (API
    // surface, storage latency) — exactly what an operator needs when the
    // crawl numbers look fine but the admin plane is misbehaving. Latency is
    // kept as running ms totals plus a request count (means are derived at
    // render time); no histograms, no high-cardinality labels.
    pub request_ms_total: AtomicU64,
    pub storage_write_ms_total: AtomicU64,
    pub storage_writes: AtomicU64,
    pub api_requests_2xx: AtomicU64,
    pub api_requests_4xx: AtomicU64,
    pub api_requests_5xx: AtomicU64,
    pub api_request_ms_total: AtomicU64,
    /// robots.txt answered from the shared (redis) politeness cache in
    /// distributed mode — cache hits, not network fetches.
    pub robots_cache_hits: AtomicU64,
    /// Written records keyed by target id — the one labeled breakdown we
    /// expose. The label set is bounded by `config.targets`, so cardinality
    /// is a config decision, not a runtime surprise.
    pub per_target_written: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    pub alerts_fired: AtomicU64,
    pub alerts_notified: AtomicU64,
    // ---- managed customer API (`/v1/*`) --------------------------------
    // A second, revenue-bearing surface on the same port. These counters are
    // what an operator reads when a tenant says "your API is slow": they
    // separate "the customer asked for too much" (rate/quota denials) from
    // "we are failing to deliver" (5xx). No tenant id, key prefix or URL is
    // ever a label — the label set stays bounded.
    /// `/v1/*` requests by status class (`2xx`/`4xx`/`5xx`).
    pub managed_api_requests: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
    /// Requests refused by a per-key rate or concurrency limit.
    pub api_rate_limited_total: AtomicU64,
    /// Requests refused by a monthly quota.
    pub api_quota_denied_total: AtomicU64,
    /// Usage units billed, summed over every charged request.
    pub api_usage_units_total: AtomicU64,
    /// Asynchronous jobs queued or running (a gauge).
    pub api_jobs_active: AtomicU64,
    /// Webhook deliveries by result (`success`/`retry`/`dead_letter`/`refused`).
    pub webhook_deliveries: std::sync::Mutex<std::collections::BTreeMap<String, u64>>,
}

impl Metrics {
    fn add(c: &AtomicU64, v: u64) {
        c.fetch_add(v, Ordering::Relaxed);
    }
    fn inc(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }

    pub fn request_started(&self) {
        Self::inc(&self.requests_total);
    }
    pub fn request_ok(&self) {
        Self::inc(&self.requests_ok);
    }
    pub fn request_retried(&self) {
        Self::inc(&self.requests_retry);
    }
    pub fn request_blocked(&self) {
        Self::inc(&self.requests_blocked);
    }
    pub fn note_backoff_ms(&self, ms: u64) {
        Self::add(&self.backoff_sleep_ms_total, ms);
    }
    pub fn count_politeness_wait(&self, _host: &str, kind: &str) {
        match kind {
            "sleep" => Self::inc(&self.politeness_sleeps),
            "permit" => Self::inc(&self.politeness_permits),
            "host_lock" => Self::inc(&self.politeness_host_locks),
            _ => {}
        }
    }
    pub fn revalidated(&self) {
        Self::inc(&self.requests_revalidated);
    }

    pub fn ckpt_written(&self) {
        Self::inc(&self.checkpoints_written);
    }

    pub fn robot_fetched(&self) {
        Self::inc(&self.robots_fetches);
    }
    pub fn robot_cache_hit(&self) {
        Self::inc(&self.robots_cache_hits);
    }
    /// One record written for `target` (per-target labeled family).
    pub fn note_target_record(&self, target: &str) {
        if let Ok(mut m) = self.per_target_written.lock() {
            *m.entry(target.to_string()).or_default() += 1;
        }
    }
    pub fn robot_denied(&self) {
        Self::inc(&self.robots_denials);
    }
    pub fn page_fetched(&self) {
        Self::inc(&self.pages_fetched);
    }
    pub fn browser_session_created(&self) {
        Self::inc(&self.browser_sessions_created);
    }
    pub fn browser_challenge_detected(&self) {
        Self::inc(&self.browser_challenges_detected);
    }
    pub fn browser_action_completed(&self) {
        Self::inc(&self.browser_actions_completed);
    }
    pub fn selector_records_previewed(&self, count: usize) {
        Self::add(&self.selector_records_previewed, count.min(100_000) as u64);
    }
    /// One acquisition refused by a bounded budget (see `crate::budget`).
    pub fn note_budget_denied(&self, scope: BudgetScope) {
        match scope {
            BudgetScope::Proxy => Self::add(&self.budget_denied_proxy, 1),
            BudgetScope::Records => Self::add(&self.budget_denied_records, 1),
        }
    }

    pub fn record_extracted(&self) {
        Self::inc(&self.records_extracted);
    }
    pub fn record_deduped(&self) {
        Self::inc(&self.records_deduped);
    }
    pub fn record_written(&self) {
        Self::inc(&self.records_written);
    }
    pub fn domain_abandoned(&self) {
        Self::inc(&self.domains_abandoned);
    }
    pub fn job_completed(&self) {
        Self::inc(&self.jobs_completed);
    }
    pub fn job_failed(&self) {
        Self::inc(&self.jobs_failed);
    }
    pub fn queue_rejected(&self) {
        Self::inc(&self.queue_rejected);
    }
    pub fn breaker_rejected(&self) {
        Self::inc(&self.breaker_rejected);
    }
    pub fn retry_budget_denied(&self) {
        Self::inc(&self.retry_budget_denied);
    }
    pub fn record_quarantined(&self) {
        Self::inc(&self.records_quarantined);
    }
    pub fn schema_drift(&self) {
        Self::inc(&self.schema_drifts);
    }
    /// Transform/schema failures observed while applying a record's
    /// pipeline. Counted separately from `schema_drifts`: a drift is a
    /// repaired value, a failure is a field the pipeline could not fix
    /// (quarantined or written raw).
    pub fn pipeline_failures(&self, n: usize) {
        Self::add(&self.pipeline_failures, n as u64);
    }
    /// Wall time of one completed fetch (any outcome), for mean request
    /// latency alongside the existing success/failure counts.
    pub fn note_request_ms(&self, ms: u64) {
        Self::add(&self.request_ms_total, ms);
    }
    /// One sink write and the time it took (fsync/db roundtrip included).
    pub fn note_storage_write(&self, ms: u64) {
        Self::add(&self.storage_write_ms_total, ms);
        Self::inc(&self.storage_writes);
    }
    /// Admin-API request accounting: status class (bounded label) + duration.
    pub fn note_api_request(&self, status: u16, ms: u64) {
        let c = match status {
            200..=399 => &self.api_requests_2xx,
            400..=499 => &self.api_requests_4xx,
            _ => &self.api_requests_5xx,
        };
        Self::inc(c);
        Self::add(&self.api_request_ms_total, ms);
    }
    /// Alert accounting: firings vs webhook deliveries (suppressed ones are
    /// fired-but-not-notified; the difference is what dedup/cooldown saved).
    /// Record one `/v1/*` request in the status-class breakdown.
    pub fn note_managed_request(&self, status: u16) {
        let class = if status >= 500 {
            "5xx"
        } else if status >= 400 {
            "4xx"
        } else {
            "2xx"
        };
        if let Ok(mut m) = self.managed_api_requests.lock() {
            *m.entry(class.to_string()).or_insert(0) += 1;
        }
    }

    pub fn note_rate_limited(&self) {
        Self::inc(&self.api_rate_limited_total);
    }

    pub fn note_quota_denied(&self) {
        Self::inc(&self.api_quota_denied_total);
    }

    pub fn note_usage_units(&self, units: u32) {
        Self::add(&self.api_usage_units_total, u64::from(units));
    }

    /// `delta` is +1 when a job starts and -1 when it reaches a terminal
    /// state; saturating so a double-decrement cannot underflow the gauge.
    pub fn note_job_active(&self, delta: i64) {
        if delta >= 0 {
            self.api_jobs_active
                .fetch_add(delta as u64, Ordering::Relaxed);
        } else {
            self.api_jobs_active
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                    Some(v.saturating_sub(delta.unsigned_abs()))
                })
                .ok();
        }
    }

    pub fn note_webhook_delivery(&self, result: &str) {
        if let Ok(mut m) = self.webhook_deliveries.lock() {
            *m.entry(result.to_string()).or_insert(0) += 1;
        }
    }

    pub fn note_alert_fired(&self, n: usize) {
        Self::add(&self.alerts_fired, n as u64);
    }
    pub fn note_alert_notified(&self) {
        Self::inc(&self.alerts_notified);
    }

    /// Prometheus text exposition — the contract Grafana dashboards read.
    pub fn render_prometheus(&self) -> String {
        use std::fmt::Write as _;
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let mut s = String::new();
        let rows = [
            (
                "harvest_requests_total",
                "requests sent",
                g(&self.requests_total),
            ),
            (
                "harvest_requests_ok",
                "successful responses",
                g(&self.requests_ok),
            ),
            (
                "harvest_requests_retry",
                "retries attempted",
                g(&self.requests_retry),
            ),
            (
                "harvest_requests_blocked",
                "401/403/429/451 responses",
                g(&self.requests_blocked),
            ),
            (
                "harvest_backoff_sleep_ms_total",
                "ms spent honoring backoff",
                g(&self.backoff_sleep_ms_total),
            ),
            (
                "harvest_politeness_sleeps",
                "rate-limit sleeps",
                g(&self.politeness_sleeps),
            ),
            (
                "harvest_politeness_permits",
                "permits granted",
                g(&self.politeness_permits),
            ),
            (
                "harvest_robots_fetches",
                "robots.txt fetches",
                g(&self.robots_fetches),
            ),
            (
                "harvest_checkpoints_written",
                "checkpoint snapshots written",
                g(&self.checkpoints_written),
            ),
            (
                "harvest_requests_revalidated",
                "conditional GETs answered 304",
                g(&self.requests_revalidated),
            ),
            (
                "harvest_robots_denials",
                "URLs skipped by robots",
                g(&self.robots_denials),
            ),
            (
                "harvest_pages_fetched",
                "pages fetched and parsed",
                g(&self.pages_fetched),
            ),
            (
                "harvest_records_extracted",
                "records extracted",
                g(&self.records_extracted),
            ),
            (
                "harvest_records_deduped",
                "records dropped by fingerprint",
                g(&self.records_deduped),
            ),
            (
                "harvest_browser_sessions_created",
                "browser sessions created",
                g(&self.browser_sessions_created),
            ),
            (
                "harvest_browser_challenges_detected",
                "observable browser challenges detected",
                g(&self.browser_challenges_detected),
            ),
            (
                "harvest_browser_actions_completed",
                "browser workflow actions completed",
                g(&self.browser_actions_completed),
            ),
            (
                "harvest_selector_records_previewed",
                "records returned by selector previews",
                g(&self.selector_records_previewed),
            ),
            (
                "harvest_records_written",
                "records persisted",
                g(&self.records_written),
            ),
            (
                "harvest_domains_abandoned",
                "domains cut by circuit breaker",
                g(&self.domains_abandoned),
            ),
            (
                "harvest_jobs_completed",
                "crawl jobs finished successfully",
                g(&self.jobs_completed),
            ),
            (
                "harvest_jobs_failed",
                "crawl jobs failed (after retries / policy)",
                g(&self.jobs_failed),
            ),
            (
                "harvest_queue_rejected",
                "URLs refused by the bounded scheduler queue",
                g(&self.queue_rejected),
            ),
            (
                "harvest_breaker_rejected",
                "jobs skipped while a transport circuit was open",
                g(&self.breaker_rejected),
            ),
            (
                "harvest_retry_budget_denied",
                "retries refused by the per-minute retry budget",
                g(&self.retry_budget_denied),
            ),
            (
                "harvest_records_quarantined",
                "records diverted to quarantine by schema enforcement",
                g(&self.records_quarantined),
            ),
            (
                "harvest_schema_drifts",
                "field values coerced or failing their declared type",
                g(&self.schema_drifts),
            ),
            (
                "harvest_pipeline_failures",
                "pipeline fields that could not be repaired (drift excluded)",
                g(&self.pipeline_failures),
            ),
            (
                "harvest_request_ms_total",
                "total ms spent on fetches (divide by harvest_requests_total for the mean)",
                g(&self.request_ms_total),
            ),
            (
                "harvest_storage_write_ms_total",
                "total ms spent in sink writes",
                g(&self.storage_write_ms_total),
            ),
            (
                "harvest_storage_writes",
                "sink writes completed",
                g(&self.storage_writes),
            ),
            (
                "harvest_alerts_fired",
                "alert conditions that fired across this process",
                g(&self.alerts_fired),
            ),
            (
                "harvest_alerts_notified",
                "alert webhooks actually delivered after dedup/cooldown",
                g(&self.alerts_notified),
            ),
            (
                "harvest_robots_cache_hits",
                "robots.txt answers served from the shared distributed cache",
                g(&self.robots_cache_hits),
            ),
        ];
        for (name, help, v) in rows {
            let _ = writeln!(s, "# HELP {name} {help}\n# TYPE {name} counter\n{name} {v}");
        }
        // One label, fixed and bounded: the status class. Never URL, path,
        // token, or client input — the exposition stays low-cardinality.
        let _ = writeln!(
            s,
            "# HELP harvest_api_requests_total admin API requests by status class\n\
             # TYPE harvest_api_requests_total counter\n\
             harvest_api_requests_total{{class=\"2xx\"}} {}\n\
             harvest_api_requests_total{{class=\"4xx\"}} {}\n\
             harvest_api_requests_total{{class=\"5xx\"}} {}\n\
             # HELP harvest_budget_denied_total acquisitions refused by a bounded resource budget\n\
             # TYPE harvest_budget_denied_total counter\n\
             harvest_budget_denied_total{{scope=\"proxy\"}} {}\n\
             harvest_budget_denied_total{{scope=\"records\"}} {}",
            g(&self.api_requests_2xx),
            g(&self.api_requests_4xx),
            g(&self.api_requests_5xx),
            g(&self.budget_denied_proxy),
            g(&self.budget_denied_records),
        );
        let _ = writeln!(
            s,
            "# HELP harvest_api_request_ms_total total ms spent serving admin API requests\n\
             # TYPE harvest_api_request_ms_total counter\n\
             harvest_api_request_ms_total {}",
            g(&self.api_request_ms_total),
        );
        let v1_rows: String = self
            .managed_api_requests
            .lock()
            .map(|m| {
                m.iter()
                    .map(|(k, n)| {
                        format!("harvest_managed_api_requests_total{{status=\"{k}\"}} {n}")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if !v1_rows.is_empty() {
            let _ = writeln!(
                s,
                "# HELP harvest_managed_api_requests_total managed /v1 requests by status class\n\
                 # TYPE harvest_managed_api_requests_total counter\n{v1_rows}\n"
            );
        }
        let _ = writeln!(
            s,
            "# HELP harvest_api_rate_limited_total requests refused by a per-key rate or concurrency limit\n\
             # TYPE harvest_api_rate_limited_total counter\n\
             harvest_api_rate_limited_total {}\n\
             # HELP harvest_api_quota_denied_total requests refused by a monthly quota\n\
             # TYPE harvest_api_quota_denied_total counter\n\
             harvest_api_quota_denied_total {}\n\
             # HELP harvest_api_usage_units_total usage units billed\n\
             # TYPE harvest_api_usage_units_total counter\n\
             harvest_api_usage_units_total {}\n\
             # HELP harvest_api_jobs_active asynchronous jobs queued or running\n\
             # TYPE harvest_api_jobs_active gauge\n\
             harvest_api_jobs_active {}",
            g(&self.api_rate_limited_total),
            g(&self.api_quota_denied_total),
            g(&self.api_usage_units_total),
            g(&self.api_jobs_active),
        );
        let wh_rows: String = self
            .webhook_deliveries
            .lock()
            .map(|m| {
                m.iter()
                    .map(|(k, n)| format!("harvest_webhook_deliveries_total{{result=\"{k}\"}} {n}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if !wh_rows.is_empty() {
            let _ = writeln!(
                s,
                "# HELP harvest_webhook_deliveries_total signed webhook deliveries by result\n\
                 # TYPE harvest_webhook_deliveries_total counter\n{wh_rows}\n"
            );
        }
        // The per-target family appears only once a target has written:
        // an all-zero labeled dump is noise for scrapers.
        let per_target: Vec<String> = self
            .per_target_written
            .lock()
            .map(|m| {
                m.iter()
                    .map(|(t, n)| format!("harvest_target_records_written{{target=\"{t}\"}} {n}"))
                    .collect()
            })
            .unwrap_or_default();
        if !per_target.is_empty() {
            let _ = writeln!(
                s,
                "# HELP harvest_target_records_written written records per crawl target\n\
                 # TYPE harvest_target_records_written counter\n{}\n",
                per_target.join("\n")
            );
        }
        s
    }
}

impl Metrics {
    /// JSON snapshot for /stats.
    pub fn display_json(&self) -> serde_json::Value {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        serde_json::json!({
            "requests_total": g(&self.requests_total),
            "requests_ok": g(&self.requests_ok),
            "requests_retry": g(&self.requests_retry),
            "requests_blocked": g(&self.requests_blocked),
            "backoff_sleep_ms_total": g(&self.backoff_sleep_ms_total),
            "politeness_sleeps": g(&self.politeness_sleeps),
            "politeness_permits": g(&self.politeness_permits),
            "robots_fetches": g(&self.robots_fetches),
            "robots_cache_hits": g(&self.robots_cache_hits),
            "checkpoints_written": g(&self.checkpoints_written),
            "requests_revalidated": g(&self.requests_revalidated),
            "robots_denials": g(&self.robots_denials),
            "pages_fetched": g(&self.pages_fetched),
            "records_extracted": g(&self.records_extracted),
            "records_deduped": g(&self.records_deduped),
            "browser_sessions_created": g(&self.browser_sessions_created),
            "browser_challenges_detected": g(&self.browser_challenges_detected),
            "browser_actions_completed": g(&self.browser_actions_completed),
            "selector_records_previewed": g(&self.selector_records_previewed),
            "records_written": g(&self.records_written),
            "domains_abandoned": g(&self.domains_abandoned),
            "jobs_completed": g(&self.jobs_completed),
            "jobs_failed": g(&self.jobs_failed),
            "queue_rejected": g(&self.queue_rejected),
            "breaker_rejected": g(&self.breaker_rejected),
            "retry_budget_denied": g(&self.retry_budget_denied),
            "records_quarantined": g(&self.records_quarantined),
            "schema_drifts": g(&self.schema_drifts),
            "pipeline_failures": g(&self.pipeline_failures),
            "request_ms_total": g(&self.request_ms_total),
            "storage_write_ms_total": g(&self.storage_write_ms_total),
            "storage_writes": g(&self.storage_writes),
            // Labeled family in the Prometheus text; the JSON view keeps the
            // total (sum) plus the per-class breakdown below.
            "api_requests_total": g(&self.api_requests_2xx)
                + g(&self.api_requests_4xx)
                + g(&self.api_requests_5xx),
            "api_requests_2xx": g(&self.api_requests_2xx),
            "api_requests_4xx": g(&self.api_requests_4xx),
            "api_requests_5xx": g(&self.api_requests_5xx),
            "api_request_ms_total": g(&self.api_request_ms_total),
            "budget_denied_total": g(&self.budget_denied_proxy) + g(&self.budget_denied_records),
            "target_records_written": self
                .per_target_written
                .lock()
                .map(|m| serde_json::Value::Object(
                    m.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                        .collect(),
                ))
                .unwrap_or_else(|_| serde_json::json!({})),            "budget_denied_proxy": g(&self.budget_denied_proxy),
            "budget_denied_records": g(&self.budget_denied_records),
            "alerts_fired": g(&self.alerts_fired),
            "alerts_notified": g(&self.alerts_notified),
            "managed_api_requests_total": self
                .managed_api_requests
                .lock()
                .map(|m| m.values().sum::<u64>())
                .unwrap_or(0),
            "managed_api_requests_by_status": self
                .managed_api_requests
                .lock()
                .map(|m| serde_json::Value::Object(
                    m.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                        .collect(),
                ))
                .unwrap_or_else(|_| serde_json::json!({})),
            "api_rate_limited_total": g(&self.api_rate_limited_total),
            "api_quota_denied_total": g(&self.api_quota_denied_total),
            "api_usage_units_total": g(&self.api_usage_units_total),
            "api_jobs_active": g(&self.api_jobs_active),
            "webhook_deliveries_total": self
                .webhook_deliveries
                .lock()
                .map(|m| m.values().sum::<u64>())
                .unwrap_or(0),
            "webhook_deliveries_by_result": self
                .webhook_deliveries
                .lock()
                .map(|m| serde_json::Value::Object(
                    m.iter()
                        .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                        .collect(),
                ))
                .unwrap_or_else(|_| serde_json::json!({})),
        })
    }
}

/// Text summary for the CLI footer and `/stats`.
impl std::fmt::Display for Metrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let g = |c: &AtomicU64| c.load(Ordering::Relaxed);
        write!(
            f,
            "requests={} ok={} retries={} blocked={} pages={} records={} deduped={} written={} robots-denied={} backoff_ms={}",
            g(&self.requests_total),
            g(&self.requests_ok),
            g(&self.requests_retry),
            g(&self.requests_blocked),
            g(&self.pages_fetched),
            g(&self.records_extracted),
            g(&self.records_deduped),
            g(&self.records_written),
            g(&self.robots_denials),
            g(&self.backoff_sleep_ms_total),
        )
    }
}

#[cfg(test)]
mod tests;
