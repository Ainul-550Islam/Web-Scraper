//! Per-host politeness audit: the receipt a crawl leaves behind.
//!
//! When `audit_path` is set, the engine records what it actually did to each
//! host during the run — request counts by outcome, pacing waits and slept
//! milliseconds, honored `Retry-After`/block backoffs, robots.txt state and
//! `Crawl-delay`, breaker trips — alongside the *intended* policy for that
//! host (rps/burst/min-interval from the config) and the run's `policy_sha256`.
//!
//! This is deliberately **independent bookkeeping** (not a projection of the
//! Prometheus counters): the report is cross-checkable against server logs.
//! `totals.requests + robots.fetched` should equal what the site's access
//! log counted for the same window — that equality is exactly what the e2e
//! test asserts, and what makes the file usable in a dispute, not just a demo.
//!
//! The markdown renderer reads the report JSON structurally, so `harvest
//! audit --md` also formats files produced by older engine versions (v1).

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::Config;

pub const VERSION: u8 = 1;

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct RobotsStats {
    /// times robots.txt was fetched (fresh; cache hits are not counted)
    pub fetched: u64,
    /// robots.txt existed at the last fetch (false = absent => allow-all)
    pub present: bool,
    /// URLs we declined to request because the site said so
    pub denials: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crawl_delay_ms: Option<u64>,
}

#[derive(Debug, Default, Clone, Serialize, PartialEq)]
pub struct HostStats {
    pub requests: u64,
    pub ok200: u64,
    pub revalidated304: u64,
    pub not_found404: u64,
    /// 401/403/429/451 — counted by status: an audit that hides which refusal
    /// codes were hit is not an audit.
    pub blocked: BTreeMap<u16, u64>,
    pub transport_errors: u64,
    /// Pacing decisions at the request boundary.
    pub wait_permits: u64,
    pub wait_sleeps: u64,
    pub wait_slept_ms: u64,
    /// Backoff applied after refusals (Retry-After honored / breaker ramp).
    pub backoff_ms: u64,
    pub breaker_abandons: u64,
    pub records_written: u64,
    pub robots: RobotsStats,
}

#[derive(Debug, Default)]
pub struct Recorder {
    start_ts: i64,
    hosts: Mutex<BTreeMap<String, HostStats>>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            start_ts: now_secs(),
            hosts: Mutex::new(BTreeMap::new()),
        }
    }

    fn with(&self, host: &str, f: impl FnOnce(&mut HostStats)) {
        if host.is_empty() {
            return;
        }
        f(self
            .hosts
            .lock()
            .unwrap()
            .entry(host.to_string())
            .or_default());
    }

    pub fn note_request(&self, host: &str, status: u16) {
        self.with(host, |h| {
            h.requests += 1;
            match status {
                200 => h.ok200 += 1,
                304 => h.revalidated304 += 1,
                404 => h.not_found404 += 1,
                401 | 403 | 429 | 451 => {
                    *h.blocked.entry(status).or_default() += 1;
                }
                _ => {}
            }
        });
    }

    pub fn note_error(&self, host: &str) {
        self.with(host, |h| h.transport_errors += 1);
    }

    pub fn note_wait(&self, host: &str, slept: Duration) {
        let slept_ms = slept.as_millis() as u64;
        self.with(host, |h| {
            if slept.is_zero() {
                h.wait_permits += 1;
            } else {
                h.wait_sleeps += 1;
                h.wait_slept_ms += slept_ms;
            }
        });
    }

    pub fn note_backoff(&self, host: &str, ms: u64) {
        self.with(host, |h| h.backoff_ms += ms);
    }

    pub fn note_abandoned(&self, host: &str) {
        self.with(host, |h| h.breaker_abandons += 1);
    }

    pub fn note_written(&self, host: &str) {
        self.with(host, |h| h.records_written += 1);
    }

    pub fn note_robots_denial(&self, host: &str) {
        self.with(host, |h| h.robots.denials += 1);
    }

    /// A robots.txt document was fetched for this origin (not a cache hit).
    pub fn note_robots_fetched(&self, host: &str, present: bool, crawl_delay: Option<Duration>) {
        self.with(host, |h| {
            h.robots.fetched += 1;
            h.robots.present = present;
            if let Some(d) = crawl_delay {
                h.robots.crawl_delay_ms = Some(d.as_millis() as u64);
            }
        });
    }

    pub fn snapshot(&self, cfg: &Config) -> serde_json::Value {
        let hosts = self.hosts.lock().unwrap_or_else(|p| p.into_inner());
        let mut hosts_out = serde_json::Map::new();
        let mut t = serde_json::json!({
            "requests": 0u64, "ok200": 0u64, "revalidated304": 0u64, "not_found404": 0u64,
            "blocked": 0u64, "transport_errors": 0u64, "wait_sleeps": 0u64, "wait_slept_ms": 0u64, "backoff_ms": 0u64,
            "records_written": 0u64, "robots_fetched": 0u64, "robots_denials": 0u64
        });
        for (host, h) in hosts.iter() {
            let pol = cfg.policy_for(host);
            let mut v = serde_json::to_value(h).unwrap();
            v["policy"] = serde_json::json!({
                "requests_per_sec": pol.requests_per_sec,
                "burst": pol.burst,
                "min_interval_ms": pol.min_interval_ms,
            });
            for (k, s) in t.as_object_mut().unwrap().iter_mut() {
                let add: u64 = match k.as_str() {
                    "requests" => h.requests,
                    "ok200" => h.ok200,
                    "revalidated304" => h.revalidated304,
                    "not_found404" => h.not_found404,
                    "blocked" => h.blocked.values().sum(),
                    "transport_errors" => h.transport_errors,
                    "wait_sleeps" => h.wait_sleeps,
                    "wait_slept_ms" => h.wait_slept_ms,
                    "backoff_ms" => h.backoff_ms,
                    "records_written" => h.records_written,
                    "robots_fetched" => h.robots.fetched,
                    "robots_denials" => h.robots.denials,
                    _ => 0,
                };
                *s = serde_json::json!(s.as_u64().unwrap_or(0) + add);
            }
            hosts_out.insert(host.clone(), v);
        }
        serde_json::json!({
            "v": VERSION,
            "run": cfg.name,
            "policy_sha256": cfg.hash(),
            "user_agent": cfg.user_agent,
            "respect_robots": cfg.respect_robots,
            "window": { "start_ts": self.start_ts, "end_ts": now_secs() },
            "hosts": hosts_out,
            "totals": t,
        })
    }

    /// Atomic write of the final report; returns it for logging/testing.
    pub fn save(&self, cfg: &Config, path: &Path) -> Result<serde_json::Value> {
        let report = self.snapshot(cfg);
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p).ok();
        }
        let body = serde_json::to_string_pretty(&report).context("serialize audit report")?;
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, format!("{body}\n"))
            .with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, path).with_context(|| format!("rename -> {}", path.display()))?;
        Ok(report)
    }
}

/// Render the report JSON (any v1 file) as a compact markdown receipt.
pub fn render_markdown(r: &serde_json::Value) -> String {
    let mut o = String::new();
    let g = |v: &'_ serde_json::Value, k: &str| -> String {
        v.get(k)
            .map(|x| x.to_string().trim_matches('"').to_string())
            .unwrap_or_default()
    };
    o.push_str(&format!("# Politeness audit — run {}\n\n", g(r, "run")));
    o.push_str(&format!(
        "- policy sha256: `{}`\n- user-agent: `{}`\n- robots enforced: {}\n- window: {} → {} ({})\n\n",
        g(r, "policy_sha256"),
        g(r, "user_agent"),
        r["respect_robots"].as_bool().unwrap_or(false),
        g(&r["window"], "start_ts"),
        g(&r["window"], "end_ts"),
        (r["window"]["end_ts"].as_i64().unwrap_or(0) - r["window"]["start_ts"].as_i64().unwrap_or(0)),
    ));
    if let Some(t) = r.get("totals") {
        o.push_str(&format!(
            "**Totals** — {} requests ({} ok, {} revalidated, {} not-found, {} blocked, {} transport errors), \
             {} pacing sleeps totalling {} ms, {} ms refused-backoff, {} records written, \
             robots fetched {} / denials honored {}.\n\n",
            g(t, "requests"), g(t, "ok200"), g(t, "revalidated304"), g(t, "not_found404"),
            g(t, "blocked"), g(t, "transport_errors"), g(t, "wait_sleeps"),
            g(t, "wait_slept_ms"), g(t, "backoff_ms"), g(t, "records_written"),
            g(t, "robots_fetched"), g(t, "robots_denials"),
        ));
    }
    o.push_str("| host | reqs | 200 | 304 | 404 | blocked | errors | waits (slept ms) | backoff ms | robots | written | policy (rps/burst/min-int) |\n");
    o.push_str("|---|---|---|---|---|---|---|---|---|---|---|---|\n");
    if let Some(obj) = r["hosts"].as_object() {
        for (host, h) in obj {
            let blocked = h["blocked"]
                .as_object()
                .map(|m| {
                    m.iter()
                        .map(|(k, v)| format!("{k}×{}", v.as_u64().unwrap_or(0)))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "—".into());
            let rob = match (
                h["robots"]["present"].as_bool(),
                h["robots"]["denials"].as_u64(),
            ) {
                (Some(true), Some(d)) if d > 0 => format!("yes, {d} denied"),
                (Some(true), _) => "yes".into(),
                _ => "absent".into(),
            };
            let delay = match h["robots"]["crawl_delay_ms"].as_u64() {
                Some(ms) => format!(" (delay {ms}ms honored)"),
                None => String::new(),
            };
            let pol = &h["policy"];
            o.push_str(&format!(
                "| {host} | {} | {} | {} | {} | {blocked} | {} | {} ({} ms) | {} | {rob}{delay} | {} | {}/{} · {} ms |\n",
                g(h, "requests"), g(h, "ok200"), g(h, "revalidated304"), g(h, "not_found404"),
                g(h, "transport_errors"),
                (h["wait_sleeps"].as_u64().unwrap_or(0) + h["wait_permits"].as_u64().unwrap_or(0)),
                g(h, "wait_slept_ms"), g(h, "backoff_ms"), g(h, "records_written"),
                g(pol, "requests_per_sec"), g(pol, "burst"), g(pol, "min_interval_ms"),
            ));
        }
    }
    o.push_str("\n*Counts are what the engine actually sent, per host, for this run — cross-checkable against the site's access log.*\n");
    o
}

#[cfg(test)]
mod tests;
