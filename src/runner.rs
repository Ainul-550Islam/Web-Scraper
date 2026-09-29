//! Scheduler loop + run-history log.
//!
//! `serve` keeps the proxy pool, admin dashboard and history file alive
//! across runs; each run gets a fresh `Metrics` so dashboard cards show the
//! current run while `out/runs.jsonl` is the cumulative audit trail.
//! Ctrl-c finishes the *current* run before exiting — never mid-request.
//!
//! Every history entry embeds `config_sha256`: over time this is a cheap,
//! verifiable attestation trail ("run X at time T executed policy H").

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde_json::{json, Value};

use crate::admin;
use crate::config::Config;
use crate::engine::{self, Summary};
use crate::metrics::Metrics;
use crate::proxy::ProxyPool;

pub fn history_entry(cfg: &Config, summary: &Summary, status: &str) -> Value {
    json!({
        "run": cfg.name,
        "ts": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "status": status,
        "duration_ms": summary.duration_ms,
        "config_sha256": cfg.hash(),
        "targets": cfg.targets.len(),
        "respect_robots": cfg.respect_robots,
        "summary": summary,
    })
}

/// Append one line to the JSONL history log (creates parents as needed).
pub fn record_history(cfg: &Config, entry: &Value) -> Result<()> {
    let path = &cfg.schedule.history_path;
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p).ok();
    }
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(f, "{entry}")?;
    Ok(())
}

/// Scheduler: repeated runs every `schedule.every_secs` (0 = back-to-back),
/// `schedule.runs` times (0 = until stopped). Errors are recorded, not fatal:
/// one bad run never kills the loop.
pub async fn serve(cfg: Config, stop: Arc<AtomicBool>) -> Result<usize> {
    let cfg = Arc::new(cfg);
    let pool = Arc::new(ProxyPool::new(&cfg.proxies));
    let runtime_slot = crate::runtime::RuntimeSlot::default();
    let state = Arc::new(admin::AdminState::new(
        Arc::new(Metrics::default()),
        pool.clone(),
        cfg.schedule.history_path.clone(),
        cfg.admin_token.clone(),
    ));
    state.attach_runtime(runtime_slot.clone());
    state.with_config(cfg.clone(), cfg.source_path.clone());
    let admin_task = if cfg.metrics_addr.is_empty() {
        None
    } else {
        Some(tokio::spawn(admin::serve(
            cfg.metrics_addr.clone(),
            state.clone(),
        )))
    };

    let mut completed = 0usize;
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let metrics = Arc::new(Metrics::default());
        state.set_metrics(metrics.clone());
        state.set_running(true);
        state.record_ops(
            "run_start",
            json!({"run": cfg.name, "every_secs": cfg.schedule.every_secs}),
        );
        let started = Instant::now();
        let outcome = engine::run_with(
            cfg.clone(),
            engine::RunCtx {
                metrics,
                pool: pool.clone(),
                runtime: runtime_slot.clone(),
            },
            Some(stop.clone()),
        )
        .await;
        state.set_running(false);

        let (entry, alert_summary, hard_err) = match outcome {
            Ok(summary) => (history_entry(&cfg, &summary, "ok"), summary, None),
            Err(e) => {
                let empty = Summary {
                    duration_ms: started.elapsed().as_millis() as u64,
                    ..Default::default()
                };
                let mut v = history_entry(&cfg, &empty, "error");
                v["error"] = e.to_string().into();
                (v, empty, Some(e))
            }
        };
        record_history(&cfg, &entry)?;
        let status = entry["status"].as_str().unwrap_or("ok").to_string();
        // The admin state's ledger dedups/suppresses across scheduled runs
        // (cooldown, severity floor, operator acks) and its metrics count
        // fired vs delivered for /stats and /alerts.
        let delivered = crate::alerts::check_and_alert_with(
            &cfg,
            &status,
            &alert_summary,
            Some(&state.alerts_ledger()),
            Some(&state.metrics()),
        )
        .await;
        state.record_ops(
            "run_end",
            json!({
                "status": status,
                "written": alert_summary.written,
                "quarantined": alert_summary.quarantined,
                "duration_ms": alert_summary.duration_ms,
                "alerts_fired": delivered.len(),
            }),
        );
        completed += 1;
        tracing::info!(completed, status = %entry["status"], "scheduled run recorded");
        if let Some(e) = hard_err {
            tracing::warn!(err = %e, "run failed; scheduler continues");
        }

        if cfg.schedule.runs > 0 && completed >= cfg.schedule.runs {
            break;
        }
        // sleep in ticks so ctrl-c is responsive between runs
        let total = Duration::from_secs(cfg.schedule.every_secs);
        let t0 = Instant::now();
        while t0.elapsed() < total {
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let rem = total - t0.elapsed();
            tokio::time::sleep(rem.min(Duration::from_millis(200))).await;
        }
    }

    if let Some(t) = admin_task {
        t.abort();
    }
    state.record_ops("serve_stop", json!({"completed": completed}));
    Ok(completed)
}

#[cfg(test)]
mod tests;
