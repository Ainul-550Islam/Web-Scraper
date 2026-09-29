//! Run-level SLO alerts. After every run (one-shot `run` or scheduled
//! `serve`), `evaluate` checks the summary against `alerts.*` thresholds; if
//! anything fires, `notify` POSTs a compact JSON payload to the webhook.
//! Alert failures are logged, never fatal — an unreachable Slack must not
//! destroy a completed collection.
//!
//! The [`AlertLedger`] sits between evaluation and delivery: it tracks the
//! lifecycle of every event kind (fire count, last notify, ack state),
//! applies `alerts.cooldown_secs` and `alerts.min_severity`, and lets an
//! operator acknowledge an event through the admin API to silence everything
//! below `error` severity until the condition resolves itself.

use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::engine::Summary;

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Alert severity; ordering matters (`min_severity` is a floor).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    #[default]
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn parse(s: &str) -> Severity {
        match s {
            "error" => Severity::Error,
            "warning" => Severity::Warning,
            _ => Severity::Info,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warning => "warning",
            Severity::Error => "error",
        }
    }
}

/// One fired condition, typed. `reason` keeps the human string the webhook
/// contract has always sent; `event` is the bounded category the ledger and
/// the admin API key on.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AlertDetail {
    pub event: String,
    pub severity: Severity,
    pub reason: String,
}

/// Full evaluation; `evaluate` is the reason-string projection of this.
pub fn evaluate_detailed(cfg: &Config, status: &str, summary: &Summary) -> Vec<AlertDetail> {
    let a = &cfg.alerts;
    if !a.enabled() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut push = |event: &str, severity: Severity, reason: String| {
        out.push(AlertDetail {
            event: event.to_string(),
            severity,
            reason,
        })
    };
    if status != "ok" && a.wants("run_error") {
        push("run_error", Severity::Error, format!("run_error:{status}"));
    }
    if a.quarantine_threshold > 0
        && a.wants("quarantine_surge")
        && summary.quarantined >= a.quarantine_threshold
    {
        push(
            "quarantine_surge",
            Severity::Warning,
            format!(
                "quarantine_surge:{}>={}",
                summary.quarantined, a.quarantine_threshold
            ),
        );
    }
    // Revalidations (304) count toward data-delivery SLOs: a repeat crawl
    // that confirms "nothing changed" IS the healthy steady state of an
    // incremental pipeline — it must not page anyone.
    let written_eff = summary.written + summary.revalidated;
    if a.wants("no_data") && written_eff < a.min_records.max(1) {
        push(
            "no_data",
            Severity::Warning,
            format!("no_data:written<{}", a.min_records.max(1)),
        );
    }
    for (tid, slo) in &a.per_target {
        let Some(t) = summary.per_target.get(tid) else {
            continue;
        };
        if a.wants("no_data") && t.written + t.revalidated < slo.min_records.max(1) {
            push(
                "no_data",
                Severity::Warning,
                format!("no_data:target={tid}:written<{}", slo.min_records.max(1)),
            );
        }
        if a.wants("high_block_rate") && slo.max_block_rate > 0.0 {
            let denom = t.written + t.blocked + t.failed;
            if denom > 0 && (t.blocked + t.failed) as f64 / denom as f64 >= slo.max_block_rate {
                push(
                    "high_block_rate",
                    Severity::Warning,
                    format!("high_block_rate:target={tid}"),
                );
            }
        }
    }
    if a.wants("high_block_rate") && a.max_block_rate > 0.0 && summary.requests_total > 0 {
        let rate = summary.blocked as f64 / summary.requests_total as f64;
        if rate >= a.max_block_rate {
            push(
                "high_block_rate",
                Severity::Warning,
                format!(
                    "high_block_rate:{:.0}%>={:.0}%",
                    rate * 100.0,
                    a.max_block_rate * 100.0
                ),
            );
        }
    }
    out
}

/// Reason strings; empty = healthy (or alerts disabled).
pub fn evaluate(cfg: &Config, status: &str, summary: &Summary) -> Vec<String> {
    evaluate_detailed(cfg, status, summary)
        .into_iter()
        .map(|a| a.reason)
        .collect()
}

/// Per-event lifecycle state. The ledger is what turns "the threshold was
/// crossed" into "ops knows about it and it is not screaming into the
/// webhook every 30 seconds".
#[derive(Debug, Clone, Serialize)]
pub struct AlertState {
    pub event: String,
    pub fires: u64,
    pub notified: u64,
    pub suppressed: u64,
    pub last_reason: String,
    pub last_severity: String,
    /// 0 = never fired.
    pub last_fire_ts: u64,
    pub last_notify_ts: u64,
    /// An operator acknowledged this event via the admin API; everything
    /// below `error` severity stays silenced until the event resolves.
    pub acknowledged: bool,
}

#[derive(Debug, Default)]
struct Entry {
    fires: u64,
    notified: u64,
    suppressed: u64,
    last_reason: String,
    last_severity: Severity,
    last_fire_ts: u64,
    last_notify_ts: u64,
    acked: bool,
}

/// Bounded by the number of event kinds (a fixed small set) — it cannot grow
/// with traffic.
#[derive(Debug, Default)]
pub struct AlertLedger {
    entries: Mutex<BTreeMap<String, Entry>>,
}

impl AlertLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record firings, return the subset that should be delivered now.
    /// Suppression rules, in order: severity floor (`alerts.min_severity`),
    /// operator ack (silences < error until the event stops firing), then
    /// the per-event cooldown (`alerts.cooldown_secs`). An event that stops
    /// firing clears its own state (acked or not) — recovery is automatic,
    /// there is no manual "resolve" call to forget.
    pub fn note(&self, cfg: &Config, fired: &[AlertDetail]) -> Vec<AlertDetail> {
        self.note_at(cfg, now_secs(), fired)
    }

    /// Clock-injectable core (tests, and anything that must not sleep).
    pub fn note_at(&self, cfg: &Config, now: u64, fired: &[AlertDetail]) -> Vec<AlertDetail> {
        let floor = Severity::parse(&cfg.alerts.min_severity);
        let cooldown = cfg.alerts.cooldown_secs;
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let live: BTreeSet<&str> = fired.iter().map(|a| a.event.as_str()).collect();
        // Resolution sweep first: events not seen this round drop out.
        g.retain(|event, _| live.contains(event.as_str()));
        let mut due = Vec::new();
        for a in fired {
            let e = g.entry(a.event.clone()).or_default();
            e.fires += 1;
            e.last_fire_ts = now;
            e.last_reason.clone_from(&a.reason);
            e.last_severity = a.severity;
            let deliver = a.severity >= floor
                && !(e.acked && a.severity < Severity::Error)
                && (cooldown == 0 || now.saturating_sub(e.last_notify_ts) >= cooldown);
            if deliver {
                e.notified += 1;
                e.last_notify_ts = now;
                due.push(a.clone());
            } else {
                e.suppressed += 1;
            }
        }
        due
    }

    /// Acknowledge an event (exact name, or "*" for all). Returns the number
    /// of live entries the ack touched.
    pub fn ack(&self, event: &str) -> usize {
        let mut g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        if event == "*" {
            let n = g.len();
            for e in g.values_mut() {
                e.acked = true;
            }
            n
        } else {
            match g.get_mut(event) {
                Some(e) => {
                    e.acked = true;
                    1
                }
                None => 0,
            }
        }
    }

    pub fn snapshot(&self) -> Vec<AlertState> {
        let g = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        g.iter()
            .map(|(event, e)| AlertState {
                event: event.clone(),
                fires: e.fires,
                notified: e.notified,
                suppressed: e.suppressed,
                last_reason: e.last_reason.clone(),
                last_severity: e.last_severity.as_str().to_string(),
                last_fire_ts: e.last_fire_ts,
                last_notify_ts: e.last_notify_ts,
                acknowledged: e.acked,
            })
            .collect()
    }
}

pub async fn notify(cfg: &Config, status: &str, reasons: &[String], summary: &Summary) {
    notify_with(cfg, status, reasons, &[], summary).await
}

/// Delivery with the typed detail list attached. `reasons` stays the wire
/// contract's string array; `alerts` (when non-empty) adds the structured
/// {event, severity, reason} records for webhook consumers that route on
/// them. Both describe the same firing set.
pub async fn notify_with(
    cfg: &Config,
    status: &str,
    reasons: &[String],
    alerts: &[AlertDetail],
    summary: &Summary,
) {
    if reasons.is_empty() {
        return;
    }
    let Some(url) = cfg.alerts.webhook_url.as_deref() else {
        return;
    };
    let ts = now_secs();
    let mut body = json!({
        "run": cfg.name,
        "ts": ts,
        "status": status,
        "reasons": reasons,
        "summary": summary,
        "policy_sha256": cfg.hash(),
    });
    if !alerts.is_empty() {
        body["alerts"] = serde_json::to_value(alerts).unwrap_or_else(|_| json!([]));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .user_agent("harvest-alerts/0.1")
        .build();
    match client {
        Ok(c) => match c.post(url).json(&body).send().await {
            Ok(r) if r.status().is_success() => {
                tracing::warn!(webhook = %crate::config::redact_url_origin(url), reasons = ?reasons, "SLO alert fired (webhook notified)")
            }
            Ok(r) => {
                tracing::warn!(webhook = %crate::config::redact_url_origin(url), status = r.status().as_u16(), "alert webhook failed")
            }
            Err(e) => {
                tracing::warn!(webhook = %crate::config::redact_url_origin(url), err = %e, "alert webhook unreachable")
            }
        },
        Err(e) => tracing::warn!(err = %e, "alert client build failed"),
    }
}

/// evaluate + notify in one call; used by the one-shot `run` path, where a
/// fresh process-local ledger means cooldown/ack have no history to act on
/// (every firing notifies — exactly the pre-ledger behavior).
pub async fn check_and_alert(cfg: &Config, status: &str, summary: &Summary) {
    check_and_alert_with(cfg, status, summary, None, None).await;
}

/// Long-lived path (scheduler): the caller's [`AlertLedger`] dedups and
/// suppresses across runs, and [`crate::metrics::Metrics`] gets the
/// fired/notified accounting. Returns the delivered subset for the ops log.
pub async fn check_and_alert_with(
    cfg: &Config,
    status: &str,
    summary: &Summary,
    ledger: Option<&AlertLedger>,
    metrics: Option<&crate::metrics::Metrics>,
) -> Vec<AlertDetail> {
    let fired = evaluate_detailed(cfg, status, summary);
    if fired.is_empty() {
        return fired;
    }
    if let Some(m) = metrics {
        m.note_alert_fired(fired.len());
    }
    let due = match ledger {
        Some(l) => l.note(cfg, &fired),
        None => fired.clone(),
    };
    if due.is_empty() {
        return due;
    }
    if let Some(m) = metrics {
        m.note_alert_notified();
    }
    let reasons: Vec<String> = due.iter().map(|a| a.reason.clone()).collect();
    notify_with(cfg, status, &reasons, &due, summary).await;
    due
}

#[cfg(test)]
mod tests;
