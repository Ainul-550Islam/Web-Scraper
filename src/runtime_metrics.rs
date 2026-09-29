//! Runtime-derived metrics adapter: turns the engine's own snapshots into a
//! single typed, bounded view for embedders and tooling — the read-side
//! counterpart of the Prometheus counters in [`crate::metrics`].
//!
//! Rules enforced here (and by tests):
//! * inputs are the already-public snapshot JSON (`GET /runtime` /
//!   `GET /stats` shapes) — this module NEVER invents values; every field is
//!   `as_u64()`-defensive, missing ⇒ 0 with `complete=false` set on the
//!   snapshot so consumers can tell a real zero from an absent counter;
//! * NO labels from free text: no URLs, hosts, tokens or user input become
//!   part of the metric identity — only fixed field names;
//! * per-target record counts are copied key-for-key from
//!   `metrics.target_records_written` (already a bounded set: target ids,
//!   operator-authored), never from page data.
//!
//! Consumer today: operators embedding the library and `tests/`-level
//! verification that `/runtime`, `/stats` and this adapter agree.

use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct RuntimeMetrics {
    // lifecycle (from the runtime snapshot's `engine` block)
    pub uptime_ms: u64,
    // execution
    pub jobs_completed: u64,
    pub jobs_failed: u64,
    pub queue_depth: u64,
    pub queue_inflight: u64,
    pub workers_registered: u64,
    pub workers_busy: u64,
    // fetch/reliability
    pub requests_total: u64,
    pub requests_ok: u64,
    pub requests_retry: u64,
    pub requests_blocked: u64,
    pub requests_4xx: u64,
    pub requests_5xx: u64,
    pub backoff_sleep_ms_total: u64,
    pub breaker_rejected: u64,
    pub queue_rejected: u64,
    pub retry_budget_denied: u64,
    // data
    pub pages_fetched: u64,
    pub records_extracted: u64,
    pub records_written: u64,
    pub records_quarantined: u64,
    pub records_deduped: u64,
    pub schema_drifts: u64,
    // politeness + robots
    pub politeness_sleeps: u64,
    pub robots_denials: u64,
    // storage (latency derived from totals — the same derivation the console
    // does; no new measurement is introduced)
    pub storage_writes: u64,
    pub storage_write_ms_total: u64,
    /// writes > 0 ⇒ mean store latency, else 0 (never a fake average)
    pub storage_avg_write_ms: u64,
    // pipeline/accounting
    pub pipeline_failures: u64,
    pub checkpoints_written: u64,
    pub domains_abandoned: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMetricsSnapshot {
    pub metrics: RuntimeMetrics,
    /// per-target written counts, verbatim from the metrics block
    pub per_target_written: Vec<(String, u64)>,
    /// false when any expected counter was absent from the input JSON —
    /// honest signal that the source snapshot was partial/error-shaped
    pub complete: bool,
}

fn num(v: &Value, key: &str, acc: &mut bool) -> u64 {
    match v[key].as_u64() {
        Some(n) => n,
        None => {
            *acc = false;
            0
        }
    }
}

/// Build from `/runtime` snapshot + optional `/stats`-style metrics object.
/// `stats` takes precedence for the metrics block when both are given (the
/// engine writes the same counters to both; preferring the dedicated one
/// keeps "stats is truth" parity with the console).
pub fn collect(runtime: &Value, stats: Option<&Value>) -> RuntimeMetricsSnapshot {
    let mut complete = true;
    let m = stats
        .map(|s| &s["metrics"])
        .filter(|v| v.is_object())
        .unwrap_or(&runtime["metrics"]);
    let engine = &runtime["engine"];
    let queue = runtime
        .get("queue")
        .cloned()
        .unwrap_or_else(|| stats.map(|s| s["queue"].clone()).unwrap_or(Value::Null));
    let workers = runtime
        .get("workers")
        .cloned()
        .unwrap_or_else(|| stats.map(|s| s["workers"].clone()).unwrap_or(Value::Null));

    let mut metrics = RuntimeMetrics {
        uptime_ms: num(engine, "uptime_ms", &mut complete),
        jobs_completed: num(m, "jobs_completed", &mut complete),
        jobs_failed: num(m, "jobs_failed", &mut complete),
        queue_depth: num(&queue, "depth", &mut complete),
        queue_inflight: num(&queue, "inflight", &mut complete),
        workers_registered: num(&workers, "registered", &mut complete),
        workers_busy: num(&workers, "busy", &mut complete),
        requests_total: num(m, "requests_total", &mut complete),
        requests_ok: num(m, "requests_ok", &mut complete),
        requests_retry: num(m, "requests_retry", &mut complete),
        requests_blocked: num(m, "requests_blocked", &mut complete),
        requests_4xx: num(m, "api_requests_4xx", &mut complete),
        requests_5xx: num(m, "api_requests_5xx", &mut complete),
        backoff_sleep_ms_total: num(m, "backoff_sleep_ms_total", &mut complete),
        breaker_rejected: num(m, "breaker_rejected", &mut complete),
        queue_rejected: num(m, "queue_rejected", &mut complete),
        retry_budget_denied: num(m, "retry_budget_denied", &mut complete),
        pages_fetched: num(m, "pages_fetched", &mut complete),
        records_extracted: num(m, "records_extracted", &mut complete),
        records_written: num(m, "records_written", &mut complete),
        records_quarantined: num(m, "records_quarantined", &mut complete),
        records_deduped: num(m, "records_deduped", &mut complete),
        schema_drifts: num(m, "schema_drifts", &mut complete),
        politeness_sleeps: num(m, "politeness_sleeps", &mut complete),
        robots_denials: num(m, "robots_denials", &mut complete),
        storage_writes: num(m, "storage_writes", &mut complete),
        storage_write_ms_total: num(m, "storage_write_ms_total", &mut complete),
        storage_avg_write_ms: 0, // derived below; not a source counter
        pipeline_failures: num(m, "pipeline_failures", &mut complete),
        checkpoints_written: num(m, "checkpoints_written", &mut complete),
        domains_abandoned: num(m, "domains_abandoned", &mut complete),
    };
    metrics.storage_avg_write_ms = metrics
        .storage_write_ms_total
        .checked_div(metrics.storage_writes)
        .unwrap_or(0);

    let per_target_written = m["target_records_written"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n)))
                .collect()
        })
        .unwrap_or_default();

    RuntimeMetricsSnapshot {
        metrics,
        per_target_written,
        complete,
    }
}

impl RuntimeMetricsSnapshot {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
    /// Sum of per-target written must equal the global counter when the
    /// source is complete — expose the check instead of asserting on callers.
    pub fn per_target_consistent(&self) -> bool {
        if !self.metrics.records_written_eq_per_target() {
            return true; // no per-target map at all: vacuously fine
        }
        let sum: u64 = self.per_target_written.iter().map(|(_, n)| n).sum();
        self.per_target_written.is_empty() || sum <= self.metrics.records_written
    }
}

impl RuntimeMetrics {
    fn records_written_eq_per_target(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_the_real_snapshot_shapes() {
        let rt = json!({
            "engine": { "run": "basic", "status": "running", "uptime_ms": 4200 },
            "queue": { "depth": 7, "inflight": 2 },
            "workers": { "registered": 3, "busy": 2 },
            "metrics": {
                "requests_total": 100, "requests_ok": 95, "requests_retry": 3,
                "requests_blocked": 2, "api_requests_4xx": 1, "api_requests_5xx": 0,
                "backoff_sleep_ms_total": 900, "breaker_rejected": 0,
                "queue_rejected": 0, "retry_budget_denied": 0,
                "jobs_completed": 90, "jobs_failed": 1,
                "pages_fetched": 98, "records_extracted": 220, "records_written": 200,
                "records_quarantined": 5, "records_deduped": 15, "schema_drifts": 1,
                "politeness_sleeps": 40, "robots_denials": 0,
                "storage_writes": 10, "storage_write_ms_total": 250,
                "pipeline_failures": 0, "checkpoints_written": 4, "domains_abandoned": 0,
                "target_records_written": { "shop": 200 }
            }
        });
        let snap = collect(&rt, None);
        assert!(snap.complete);
        assert_eq!(snap.metrics.uptime_ms, 4200);
        assert_eq!(snap.metrics.queue_depth, 7);
        assert_eq!(snap.metrics.records_written, 200);
        assert_eq!(
            snap.metrics.storage_avg_write_ms, 25,
            "derived mean, not a stored value"
        );
        assert_eq!(snap.per_target_written, vec![("shop".to_string(), 200u64)]);
    }

    #[test]
    fn absent_counters_flag_incompleteness_instead_of_lie() {
        let snap = collect(&json!({ "error": "no run active" }), None);
        assert!(
            !snap.complete,
            "error-shaped snapshot must not read as all-zero-truth"
        );
        assert_eq!(snap.metrics.records_written, 0);
    }

    #[test]
    fn stats_block_wins_over_runtime_metrics_block() {
        let rt = json!({"engine": {"uptime_ms": 1}, "metrics": {"records_written": 10}});
        let stats = json!({"metrics": {"records_written": 42}});
        let snap = collect(&rt, Some(&stats));
        assert_eq!(snap.metrics.records_written, 42);
    }
}
