//! The orchestrator: bounded worker pool over a shared job queue, with
//! robots enforcement, per-domain pacing, fingerprint dedup, sink writes,
//! and an embedded admin/metrics HTTP server.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;
use url::Url;

use crate::admin;
use crate::breaker::{Breaker, BreakerEvent, Permit};
use crate::config::Config;
use crate::events::EventKind;
use crate::exec::{RequestContext, RequestPlan};
use crate::extract::{self, Extractor, JsonExtractor};
use crate::fetcher::{FetchError, Fetcher};
use crate::job::{Job, JobId};
use crate::metrics::Metrics;
use crate::policy::Politeness;
use crate::proxy::ProxyPool;
use crate::retry::RetryBudget;
use crate::robots::Robots;
use crate::runtime::{Runtime, RuntimeSlot};
use crate::sched::{CrawlQueue, Popped};
use crate::sink::{self, Record, Sink};
use crate::sitemap::{self, Sitemap};
use crate::supervisor::{CancelToken, RestartPolicy, Supervisor, TaskSet};
use crate::transform::TargetPipeline;
use scraper::{Html, Selector};

#[derive(Debug, Clone, Copy, Serialize, Default)]
pub struct TargetSummary {
    pub written: u64,
    pub blocked: u64,
    pub failed: u64,
    /// Conditional GETs answered 304 for this target (page confirmed fresh,
    /// not rewritten). Counts toward SLOs in place of `written`.
    #[serde(default)]
    pub revalidated: u64,
    /// Records diverted to the schema quarantine instead of the sink
    /// (strict mode; see `schema` module). 0 = pipeline not enforcing.
    #[serde(default)]
    pub quarantined: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Summary {
    pub pages_fetched: u64,
    pub requests_total: u64,
    pub records: u64,
    pub written: u64,
    pub deduped: u64,
    pub blocked: u64,
    pub retries: u64,
    pub robots_denied: u64,
    /// Checkpoint snapshots written during this run (0 = plain run or none
    /// configured; >0 with an unfinished run means `--resume` can continue).
    #[serde(default)]
    pub ckpt_writes: u64,
    /// Whole-run count of 304 revalidations (0 unless revalidate_store set).
    #[serde(default)]
    pub revalidated: u64,
    /// Records diverted by strict schema enforcement (see `schema`).
    #[serde(default)]
    pub quarantined: u64,
    /// Fields coerced or failing their declared type across the run
    /// (observation channel for `off`/`coerce` modes; drift never fails a
    /// crawl — it just says the source stopped matching the contract).
    #[serde(default)]
    pub schema_drifts: u64,
    pub duration_ms: u64,
    /// Per-target results (ids from config); the basis of per-target SLOs
    /// and of which DLQ entries still matter.
    pub per_target: std::collections::BTreeMap<String, TargetSummary>,
}

type RobotsMap = Arc<tokio::sync::Mutex<HashMap<String, (Instant, Arc<Robots>)>>>;

/// Stop-aware one-shot entry (used by CLI ctrl-c handling).
pub async fn run_with_stop(
    cfg: Config,
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    retry_dlq: bool,
    resume: bool,
) -> Result<Summary> {
    let cfg = Arc::new(cfg);
    let ctx = RunCtx {
        metrics: Arc::new(Metrics::default()),
        pool: Arc::new(ProxyPool::new(&cfg.proxies)),
        runtime: RuntimeSlot::default(),
    };
    let state = Arc::new(admin::AdminState::new(
        ctx.metrics.clone(),
        ctx.pool.clone(),
        cfg.schedule.history_path.clone(),
        cfg.admin_token.clone(),
    ));
    state.attach_runtime(ctx.runtime.clone());
    state.with_config(cfg.clone(), cfg.source_path.clone());
    let admin_task = if cfg.metrics_addr.is_empty() {
        None
    } else {
        Some(tokio::spawn(admin::serve(
            cfg.metrics_addr.clone(),
            state.clone(),
        )))
    };
    state.set_running(true);
    let summary = run_with_opt(cfg, ctx, stop, retry_dlq, resume).await;
    state.set_running(false);
    if let Some(t) = admin_task {
        t.abort();
    }
    summary
}

/// Per-run shared context usable by the admin dashboard.
/// `runtime`: slot filled by the engine when a run starts; the admin
/// `/runtime` and `/events` routes read through it. `Default` for embedders
/// that only care about metrics/proxies.
pub struct RunCtx {
    pub metrics: Arc<Metrics>,
    pub pool: Arc<ProxyPool>,
    pub runtime: RuntimeSlot,
}

impl Default for RunCtx {
    fn default() -> Self {
        Self {
            metrics: Arc::new(Metrics::default()),
            pool: Arc::new(ProxyPool::new(&[])),
            runtime: RuntimeSlot::default(),
        }
    }
}

/// One-shot run: fresh counters, embedded admin server for the run duration
/// (bind failure is logged, not fatal; empty `metrics_addr` disables it).
pub async fn run(cfg: Config) -> Result<Summary> {
    run_with_stop(cfg, None, false, false).await
}

/// Core collection run against a caller-owned context (used by the
/// scheduler, which keeps the admin server and pool alive between runs).
/// `stop`: when set, workers finish the in-flight job and exit — queued jobs
/// are abandoned politely (graceful drain, no mid-request kill).
pub async fn run_with(
    cfg: Arc<Config>,
    ctx: RunCtx,
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<Summary> {
    run_with_opt(cfg, ctx, stop, false, false).await
}

/// `retry_dlq`: additionally seed URLs from the dead-letter file and
/// rewrite it afterwards with only the entries that failed again (attempts
/// incremented). Entry shape: `{"url":..., "reason":..., "attempts":...}`.
/// `resume`: continue from `ckpt_path` (see [`crate::ckpt`]); refuses under
/// changed policy, deletes the file on a complete run. A fresh run removes
/// any stale checkpoint at startup.
pub async fn run_with_opt(
    cfg: Arc<Config>,
    ctx: RunCtx,
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    retry_dlq: bool,
    resume: bool,
) -> Result<Summary> {
    let started = Instant::now();
    let metrics = ctx.metrics.clone();
    let pool = ctx.pool.clone();
    // Resource budgets resolved once per run: the proxy pool learns its
    // in-flight ceiling; the record cap gates writes (0 = unlimited, and
    // an unlimited Budget costs two atomics and a branch).
    pool.set_inflight_cap(cfg.max_inflight_per_proxy);
    let record_cap = Arc::new(crate::budget::Budget::new(
        "records_per_run",
        cfg.max_records_per_run,
    ));
    let cap_announced = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // Engine-layer runtime: event bus, accounting, breakers, supervisor.
    // Created before anything can fail hard so the events log exists from
    // the first lifecycle moment of the run.
    let runtime: Arc<Runtime> = Runtime::new(&cfg, metrics.clone());
    ctx.runtime.set(runtime.clone());
    let supervisor = Supervisor::new(
        RestartPolicy {
            max_restarts: cfg.scheduling.worker_restarts,
            backoff: Duration::from_millis(250),
        },
        stop.clone()
            .map(CancelToken::from_shared)
            .unwrap_or_default(),
    );
    runtime.attach_supervisor(supervisor.clone());
    let fetcher = Arc::new(Fetcher::new(cfg.clone(), pool.clone()));
    let politeness = Arc::new(Politeness::new(&cfg));
    let (sink, sink_path) = sink::build(&cfg.output)?;
    let audit: Option<Arc<crate::audit::Recorder>> = cfg
        .audit_path
        .as_ref()
        .map(|_| Arc::new(crate::audit::Recorder::new()));
    // Load the attestation key BEFORE crawling: a run configured to sign its
    // receipts must not discover the key is missing after 20 minutes of work.
    let attester: Option<crate::sign::Key> = match &cfg.attest_key_path {
        Some(p) => Some(
            crate::sign::Key::load(p)
                .with_context(|| format!("attest_key_path {}", p.display()))?,
        ),
        None => None,
    };

    let extractors: Arc<Vec<ExtractMode>> = Arc::new(
        cfg.targets
            .iter()
            .map(|t| match t.mode.as_str() {
                "json" => Ok(ExtractMode::Json(JsonExtractor::compile(&t.extract)?)),
                _ => Ok(ExtractMode::Html(Extractor::compile(&t.extract)?)),
            })
            .collect::<Result<Vec<_>>>()?,
    );
    // Extraction pipeline: transform chains + declared schema per target.
    // `Config::validate` compiles the same thing at config time; this is
    // the runtime instance workers share (regexes compiled once per run).
    let pipelines: Arc<Vec<TargetPipeline>> = Arc::new(
        cfg.targets
            .iter()
            .map(TargetPipeline::compile)
            .collect::<Result<Vec<_>>>()?,
    );
    let robots_cache: RobotsMap = Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    let queue: Arc<CrawlQueue> = CrawlQueue::new(&cfg.scheduling, cfg.targets.len().max(1));
    runtime.attach_queue(queue.clone());
    let visited: Arc<std::sync::Mutex<Vec<HashSet<String>>>> = Arc::new(std::sync::Mutex::new(
        cfg.targets.iter().map(|_| HashSet::new()).collect(),
    ));
    let seen_fp: Arc<std::sync::Mutex<Vec<HashSet<crate::ids::RecordId>>>> = Arc::new(
        std::sync::Mutex::new(cfg.targets.iter().map(|_| HashSet::new()).collect()),
    );
    let per_target_count: Arc<Vec<AtomicUsize>> = Arc::new(
        (0..cfg.targets.len())
            .map(|_| AtomicUsize::new(0))
            .collect(),
    );
    let outstanding: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let global_pages: Arc<AtomicUsize> = Arc::new(AtomicUsize::new(0));
    let per_target: Arc<std::sync::Mutex<std::collections::BTreeMap<String, TargetSummary>>> =
        Default::default();
    let dlq_fail: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>> =
        Default::default();
    // --- checkpoint / resume (see crate::ckpt for the contract) ---------
    let mut ckpt_restored: Option<crate::ckpt::Checkpoint> = None;
    if resume {
        let Some(p) = cfg.ckpt_path.as_ref() else {
            anyhow::bail!("--resume requires `ckpt_path` in the config");
        };
        if p.exists() {
            ckpt_restored = Some(crate::ckpt::Checkpoint::load_validated(p, &cfg)?);
        } else {
            // A completed run *deletes* its checkpoint — resuming a finished
            // crawl is therefore a no-op fresh start, not an error.
            tracing::warn!(path = %p.display(), "no checkpoint file — starting fresh");
        }
    } else if let Some(p) = cfg.ckpt_path.as_ref() {
        if p.exists() {
            tracing::info!(path = %p.display(), "fresh run — removing stale checkpoint");
            std::fs::remove_file(p).ok();
        }
    }
    let ckpt: Option<Arc<CkptCtl>> = cfg.ckpt_path.as_ref().map(|p| {
        Arc::new(CkptCtl {
            cfg: cfg.clone(),
            path: p.clone(),
            gate: cfg.ckpt_every_pages.max(1),
            metrics: metrics.clone(),
            pages_since: AtomicUsize::new(0),
            queue: queue.clone(),
            visited: visited.clone(),
            seen_fp: seen_fp.clone(),
            per_target_count: per_target_count.clone(),
            global_pages: global_pages.clone(),
        })
    });
    if let Some(ck) = &ckpt_restored {
        {
            let mut v = visited.lock().unwrap_or_else(|p| p.into_inner());
            for (i, ts) in ck.targets.iter().enumerate() {
                v[i] = ts.visited.iter().cloned().collect::<HashSet<_>>();
                per_target_count[i].store(ts.written, Ordering::SeqCst);
            }
        }
        {
            let mut f = seen_fp.lock().unwrap_or_else(|p| p.into_inner());
            for (i, ts) in ck.targets.iter().enumerate() {
                // Checkpoint wire format is plain strings; entries that no
                // longer parse as fingerprints are dropped (a corrupt fp
                // must not poison the dedupe set).
                f[i] = ts
                    .fp
                    .iter()
                    .filter_map(|s| crate::ids::RecordId::parse(s).ok())
                    .collect::<HashSet<_>>();
            }
        }
        global_pages.store(ck.global_pages, Ordering::SeqCst);
        let mut restored_jobs = 0usize;
        {
            for pj in &ck.pending {
                if let Ok(u) = Url::parse(&pj.url) {
                    if pj.target < cfg.targets.len()
                        && queue.push(mk_job(&cfg, u, pj.depth, pj.target)).is_ok()
                    {
                        restored_jobs += 1;
                    }
                }
            }
        }
        outstanding.store(restored_jobs, Ordering::SeqCst);
        tracing::info!(
            pending = restored_jobs,
            global_pages = ck.global_pages,
            "resumed from checkpoint — already-enqueued URLs will not be fetched again"
        );
    }
    let resuming = ckpt_restored.is_some();
    let reval: Option<Arc<crate::reval::Store>> = cfg
        .revalidate_store
        .as_ref()
        .and_then(
            |p| match crate::reval::Store::load(p.clone(), cfg.revalidate_ttl_secs) {
                Ok(s) => Some(s),
                Err(e) => {
                    tracing::warn!(%e, "validators store unreadable; running without revalidation");
                    None
                }
            },
        )
        .map(Arc::new);
    if let Some(s) = &reval {
        tracing::info!(
            entries = s.len(),
            ttl = cfg.revalidate_ttl_secs,
            "incremental revalidation active"
        );
    }
    let dlq_ok: Arc<std::sync::Mutex<std::collections::HashSet<String>>> = Default::default();
    let mut dlq_existing: std::collections::BTreeMap<String, serde_json::Value> =
        Default::default();
    let mut retried: std::collections::HashSet<String> = Default::default();
    if retry_dlq {
        if let Some(p) = &cfg.dlq_path {
            if let Ok(txt) = std::fs::read_to_string(p) {
                for line in txt.lines() {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        if let Some(u) = v.get("url").and_then(|x| x.as_str()).map(str::to_string) {
                            retried.insert(u.clone());
                            dlq_existing.insert(u, v);
                        }
                    }
                }
            }
        }
    }

    // Sitemap-driven seeds (fetched before workers start, under the same
    // politeness + robots guarantees as regular crawl traffic).
    let seed_jobs: Vec<(usize, Url)> = if !resuming && cfg.targets.iter().any(|t| t.sitemap) {
        let sctx = SeedCtx {
            cfg: &cfg,
            fetcher: &fetcher,
            politeness: &politeness,
            metrics: &metrics,
            robots_cache: &robots_cache,
            audit: audit.clone(),
        };
        sitemap_seeds(&sctx).await
    } else {
        Vec::new()
    };

    // Seed start URLs (+ sitemaps + DLQ retries). Skipped when resuming:
    // `visited` already contains every URL the dead process queued.
    {
        if !resuming {
            for (i, t) in cfg.targets.iter().enumerate() {
                let u = Url::parse(&t.start_url)?;
                visited.lock().unwrap_or_else(|p| p.into_inner())[i].insert(dedup_key(&u));
                if queue.push(mk_job(&cfg, u, 0, i)).is_ok() {
                    outstanding.fetch_add(1, Ordering::SeqCst);
                } else {
                    metrics.queue_rejected();
                    anyhow::bail!(
                        "start URL for target {} refused by scheduling.max_queue_depth (cap below seed count)",
                        t.id
                    );
                }
            }
        }
        for (ti, u) in seed_jobs {
            if visited.lock().unwrap_or_else(|p| p.into_inner())[ti].insert(dedup_key(&u))
                && queue.push(mk_job(&cfg, u, 0, ti)).is_ok()
            {
                outstanding.fetch_add(1, Ordering::SeqCst);
            }
        }
        if retry_dlq {
            let by_id: std::collections::HashMap<&str, usize> = cfg
                .targets
                .iter()
                .enumerate()
                .map(|(i, t)| (t.id.as_str(), i))
                .collect();
            for u in &retried {
                let Ok(parsed) = Url::parse(u) else { continue };
                let host = parsed.host_str().unwrap_or_default().to_string();
                let ti = cfg
                    .targets
                    .iter()
                    .position(|t| {
                        Url::parse(&t.start_url)
                            .ok()
                            .and_then(|su| su.host_str().map(|h| h == host))
                            .unwrap_or(false)
                            && by_id.contains_key(t.id.as_str())
                    })
                    .unwrap_or(0);
                visited.lock().unwrap_or_else(|p| p.into_inner())[ti].insert(dedup_key(&parsed));
                if queue.push(mk_job(&cfg, parsed, 0, ti)).is_ok() {
                    outstanding.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    let n_workers = cfg.concurrency.clamp(1, 64);
    let mut handles = Vec::new();
    for worker in 0..n_workers {
        let ctx = WorkerCtx {
            worker_id: worker,
            cfg: cfg.clone(),
            fetcher: fetcher.clone(),
            politeness: politeness.clone(),
            pool: pool.clone(),
            sink: sink.clone(),
            metrics: metrics.clone(),
            queue: queue.clone(),
            runtime: runtime.clone(),
            supervisor: supervisor.clone(),
            retry_budget: runtime.retry_budget(),
            record_cap: record_cap.clone(),
            cap_announced: cap_announced.clone(),
            job_failed: Arc::new(AtomicBool::new(false)),
            job_fail_reason: Arc::new(std::sync::Mutex::new(String::new())),
            visited: visited.clone(),
            seen_fp: seen_fp.clone(),
            per_target_count: per_target_count.clone(),
            outstanding: outstanding.clone(),
            global_pages: global_pages.clone(),
            extractors: extractors.clone(),
            pipelines: pipelines.clone(),
            robots_cache: robots_cache.clone(),
            stop: stop.clone(),
            per_target: per_target.clone(),
            dlq_fail: dlq_fail.clone(),
            dlq_ok: dlq_ok.clone(),
            retried: Arc::new(retried.clone()),
            ckpt: ckpt.clone(),
            audit: audit.clone(),
            reval: reval.clone(),
        };
        // Supervised spawn: with `scheduling.worker_restarts = 0` (the
        // default) a panic surfaces exactly like the bare spawn did; with a
        // restart budget it is retried under backoff and counted.
        let sup = supervisor.clone();
        handles.push(tokio::spawn(async move {
            sup.supervise(worker, move || {
                let c = ctx.clone();
                async move {
                    worker_loop(c).await;
                }
            })
            .await
        }));
    }

    // Bounded background tasks (never orphaned): the checkpoint ticker
    // lives in a TaskSet and is aborted at run end; the events-log writer
    // gets its own join so the JSONL file is flushed, not killed.
    let tasks = TaskSet::new(2);
    if let Some(c) = ckpt.clone() {
        let every = c.cfg.ckpt_every_secs.max(1);
        tasks.spawn("ckpt-ticker", async move {
            loop {
                tokio::time::sleep(Duration::from_secs(every)).await;
                c.tick(true);
            }
        });
    }
    let events_writer = cfg.events_path.as_ref().map(|p| {
        let bus = runtime.bus.clone();
        let path = p.clone();
        tokio::spawn(async move { crate::runtime::events_log_writer(bus, path).await })
    });

    runtime.begin_shutdown(); // drain-phase marker: intake over, workers draining
    let mut first_err: Option<anyhow::Error> = None;
    for h in handles {
        match h.await {
            Ok(Err(msg)) => {
                if first_err.is_none() {
                    first_err = Some(anyhow::anyhow!(msg));
                }
            }
            Ok(Ok(())) => {}
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(anyhow::anyhow!("worker task died: {e}"));
                }
            }
        }
    }
    sink.flush()?;

    // Stragglers: ticker first (abort-on-zero-timeout), queue detached so
    // post-run snapshots never show a stale queue.
    let _ = tasks.shutdown(Duration::ZERO).await;
    if let Some(c) = &ckpt {
        let cancelled = stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed));
        let leftover = c.pending_count() > 0;
        if cancelled || leftover || first_err.is_some() {
            c.write_now();
            tracing::info!(
                path = %c.path.display(),
                pending = c.pending_count(),
                "checkpoint finalized — continue with `harvest run --resume`"
            );
        } else {
            std::fs::remove_file(&c.path).ok();
            tracing::info!("run complete — checkpoint removed (nothing left to resume)");
        }
    }

    if let Some(s) = &reval {
        match s.save() {
            Ok(()) => tracing::info!(entries = s.len(), "validators store saved"),
            Err(e) => {
                tracing::warn!(%e, "validators store save failed; revalidation data from this run is lost")
            }
        }
    }
    if let (Some(a), Some(p)) = (&audit, &cfg.audit_path) {
        match a.save(&cfg, p) {
            Ok(r) => tracing::info!(
                path = %p.display(),
                hosts = r["hosts"].as_object().map(|m| m.len()).unwrap_or(0),
                requests = r["totals"]["requests"].as_u64().unwrap_or(0),
                "politeness audit report written"
            ),
            Err(e) => tracing::warn!(%e, "politeness audit report write failed"),
        }
        if let Some(k) = &attester {
            match k.sign_file(p) {
                Ok(s) => tracing::info!(path = %s.display(), "audit receipt signed (HMAC-SHA256)"),
                Err(e) => tracing::warn!(%e, "audit receipt signing failed"),
            }
        }
    }

    // Queue detached first so any post-run snapshot reads a settled engine.
    runtime.detach_queue();
    let summary = Summary {
        pages_fetched: load(&metrics.pages_fetched),
        requests_total: load(&metrics.requests_total),
        records: load(&metrics.records_extracted),
        written: load(&metrics.records_written),
        deduped: load(&metrics.records_deduped),
        blocked: load(&metrics.requests_blocked),
        retries: load(&metrics.requests_retry),
        robots_denied: load(&metrics.robots_denials),
        ckpt_writes: load(&metrics.checkpoints_written),
        revalidated: load(&metrics.requests_revalidated),
        quarantined: load(&metrics.records_quarantined),
        schema_drifts: load(&metrics.schema_drifts),
        duration_ms: started.elapsed().as_millis() as u64,
        per_target: std::mem::take(&mut *per_target.lock().unwrap_or_else(|p| p.into_inner())),
    };
    if let Some(p) = &cfg.dlq_path {
        let ok = dlq_ok.lock().unwrap_or_else(|p| p.into_inner());
        let fails = dlq_fail.lock().unwrap_or_else(|p| p.into_inner());
        let mut kept: Vec<serde_json::Value> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = Default::default();
        if retry_dlq {
            for (u, v) in &dlq_existing {
                seen_urls.insert(u.clone());
                if ok.contains(u) {
                    continue; // recovered — drop from DLQ
                }
                if let Some(reason) = fails.get(u) {
                    let a = v.get("attempts").and_then(|x| x.as_u64()).unwrap_or(0) + 1;
                    if a >= cfg.dlq_max_attempts as u64 {
                        continue; // dead for good
                    }
                    let mut nv = v.clone();
                    nv["attempts"] = a.into();
                    nv["reason"] = reason.clone().into();
                    kept.push(nv);
                } else {
                    kept.push(v.clone()); // not attempted this run (drain/budget)
                }
            }
        } else {
            for (u, v) in &dlq_existing {
                seen_urls.insert(u.clone());
                kept.push(v.clone());
            }
        }
        for (u, reason) in fails.iter() {
            if seen_urls.contains(u) {
                continue;
            }
            if (cfg.dlq_max_attempts) < 1 {
                continue;
            }
            kept.push(serde_json::json!({"url": u, "reason": reason, "attempts": 1}));
        }
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let body: String = kept.iter().map(|v| format!("{v}\n")).collect();
        std::fs::write(p, body).ok();
        tracing::info!(path = %p.display(), entries = kept.len(), "dlq rewritten");
    }
    runtime.end_shutdown(&json!({
        "written": summary.written,
        "jobs_failed": load(&metrics.jobs_failed),
        "queue_left": queue.stats().depth,
        "error": first_err.is_some(),
    }));
    // Close the bus, then join the writer: every event published during the
    // run (including this one) is on disk before the run returns.
    runtime.bus.close();
    if let Some(w) = events_writer {
        match w.await {
            Ok(Ok(n)) => tracing::debug!(lines = n, "events log closed"),
            Ok(Err(e)) => tracing::warn!(%e, "events log writer failed"),
            Err(e) => tracing::warn!(err = %e, "events log writer task died"),
        }
    }
    tracing::info!(sink = %sink_path.display(), summary = ?summary, "run complete");
    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(summary)
}

/// Fetch-and-cache robots.txt per origin (TTL from config); shared by the
/// worker loop and sitemap seeding.
async fn robots_for(
    cfg: &Config,
    fetcher: &Fetcher,
    metrics: &Metrics,
    cache: &RobotsMap,
    audit: Option<&crate::audit::Recorder>,
    base: &Url,
) -> Arc<Robots> {
    let key = format!("{}://{}", base.scheme(), base.authority());
    let ttl = Duration::from_secs(cfg.robots_ttl_secs);
    {
        let g = cache.lock().await;
        if let Some((t, r)) = g.get(&key) {
            if t.elapsed() < ttl {
                return r.clone();
            }
        }
    }
    let txt = fetcher.fetch_robots(base, metrics).await;
    let robots = Arc::new(match txt {
        Some(t) => Robots::parse(&t, &ua_token(&cfg.user_agent)),
        None => Robots::default(), // no robots.txt => allow-all
    });
    if let Some(a) = audit {
        a.note_robots_fetched(
            base.host_str().unwrap_or_default(),
            robots.present,
            robots.crawl_delay,
        );
    }
    cache
        .lock()
        .await
        .insert(key, (Instant::now(), robots.clone()));
    robots
}

/// Politely fetch + flatten sitemaps for every `sitemap: true` target.
/// Returns (target index, url) seeds. Never fails the run: an unfetchable
/// or denied sitemap degrades to start-URL crawling with a warning.
struct SeedCtx<'a> {
    cfg: &'a Arc<Config>,
    fetcher: &'a Arc<Fetcher>,
    politeness: &'a Arc<Politeness>,
    metrics: &'a Arc<Metrics>,
    robots_cache: &'a RobotsMap,
    audit: Option<Arc<crate::audit::Recorder>>,
}

impl SeedCtx<'_> {
    /// robots-gate + pace + fetch one sitemap document body.
    async fn grab(&self, u: &Url) -> Option<String> {
        let host = u.host_str().unwrap_or_default().to_string();
        if self.cfg.respect_robots {
            let r = robots_for(
                self.cfg,
                self.fetcher,
                self.metrics,
                self.robots_cache,
                self.audit.as_deref(),
                &origin(u),
            )
            .await;
            if !r.allows(u.path()) {
                if let Some(a) = &self.audit {
                    a.note_robots_denial(&host);
                }
                tracing::warn!(url = %u, "sitemap refused by robots.txt; honoring");
                return None;
            }
        }
        let t = Instant::now();
        self.politeness.wait(&host, self.cfg, self.metrics).await;
        if let Some(a) = &self.audit {
            a.note_wait(&host, t.elapsed());
        }
        match self.fetcher.get(u.clone(), self.metrics).await {
            Ok(o) => {
                if let Some(a) = &self.audit {
                    a.note_request(&host, o.status);
                }
                Some(o.body)
            }
            Err(e) => {
                if let Some(a) = &self.audit {
                    a.note_error(&host);
                }
                tracing::warn!(url = %u, err = ?e, "sitemap fetch failed; continuing without it");
                None
            }
        }
    }
}

async fn sitemap_seeds(ctx: &SeedCtx<'_>) -> Vec<(usize, Url)> {
    let cfg = &ctx.cfg;
    const MAX_CHILD_DOCS: usize = 64;
    const MAX_TOTAL_URLS: usize = 5_000;

    let mut out = Vec::new();
    for (i, t) in cfg.targets.iter().enumerate() {
        if !t.sitemap {
            continue;
        }
        let Ok(start) = Url::parse(&t.start_url) else {
            continue;
        };
        let su = match &t.sitemap_url {
            Some(s) => match Url::parse(s) {
                Ok(u) => u,
                Err(_) => continue, // already validated; unreachable
            },
            None => match origin(&start).join("/sitemap.xml") {
                Ok(u) => u,
                Err(_) => continue,
            },
        };
        let Some(body) = ctx.grab(&su).await else {
            continue;
        };
        // `Url::parse` first, relative locs resolve against their document —
        // sloppy-but-live sitemaps do that, and a stray "/foo" shouldn't
        // silently drop pages that the site explicitly advertised.
        let rel = |base: &Url, s: &str| Url::parse(s).or_else(|_| base.join(s)).ok();
        let mut candidates: Vec<Url> = Vec::new();
        match sitemap::parse(&body) {
            Sitemap::Urls(v) => candidates.extend(v.iter().filter_map(|l| rel(&su, l))),
            Sitemap::Index(children) => {
                for c in children.iter().take(MAX_CHILD_DOCS) {
                    let Some(cu) = rel(&su, c) else { continue };
                    if let Some(cb) = ctx.grab(&cu).await {
                        if let Sitemap::Urls(inner) = sitemap::parse(&cb) {
                            candidates.extend(inner.iter().filter_map(|l| rel(&cu, l)));
                        } else {
                            tracing::warn!(url = %cu, "nested sitemapindex ignored (depth 1 only)");
                        }
                    }
                }
            }
        }
        let mut pushed = 0usize;
        for abs in candidates {
            if pushed >= MAX_TOTAL_URLS {
                tracing::warn!(target = %t.id, cap = MAX_TOTAL_URLS, "sitemap seed truncated at cap");
                break;
            }
            if abs.scheme() != start.scheme()
                || abs.host_str() != start.host_str()
                || abs.port() != start.port()
            {
                continue; // off-site: this target's policy scope, not ours to crawl
            }
            if !t.include.is_empty() && !t.include.iter().any(|p| abs.as_str().contains(p.as_str()))
            {
                continue;
            }
            if t.exclude.iter().any(|p| abs.as_str().contains(p.as_str())) {
                continue;
            }
            if cfg.respect_robots {
                let r = robots_for(
                    cfg,
                    ctx.fetcher,
                    ctx.metrics,
                    ctx.robots_cache,
                    ctx.audit.as_deref(),
                    &origin(&abs),
                )
                .await;
                if !r.allows(abs.path()) {
                    ctx.metrics.robot_denied();
                    if let Some(a) = &ctx.audit {
                        a.note_robots_denial(abs.host_str().unwrap_or_default());
                    }
                    continue;
                }
            }
            out.push((i, abs));
            pushed += 1;
        }
        tracing::info!(target = %t.id, urls = pushed, "sitemap seeding complete");
    }
    out
}

/// Shared checkpoint controller. Workers `tick(false)` after each record;
/// a background task `tick(true)` enforces the time interval. Snapshot
/// locking order (queue → visited → seen_fp) matches the seed block, and
/// `save` is atomic (temp+rename), so readers never see a torn file.
struct CkptCtl {
    cfg: Arc<Config>,
    path: PathBuf,
    gate: usize,
    metrics: Arc<Metrics>,
    pages_since: AtomicUsize,
    queue: Arc<CrawlQueue>,
    visited: Arc<std::sync::Mutex<Vec<HashSet<String>>>>,
    seen_fp: Arc<std::sync::Mutex<Vec<HashSet<crate::ids::RecordId>>>>,
    per_target_count: Arc<Vec<AtomicUsize>>,
    global_pages: Arc<AtomicUsize>,
}

impl CkptCtl {
    fn tick(&self, by_time: bool) {
        let n = self.pages_since.load(Ordering::SeqCst);
        if n == 0 || !(by_time || n >= self.gate) {
            return;
        }
        self.pages_since.store(0, Ordering::SeqCst);
        self.write_now();
    }

    fn pending_count(&self) -> usize {
        self.queue.len()
    }

    fn write_now(&self) {
        use crate::ckpt::{Checkpoint, PendingJob, TargetState};
        let n = self.pages_since.swap(0, Ordering::SeqCst);
        let pending = self
            .queue
            .pending_snapshot()
            .into_iter()
            .map(|(url, depth, target)| PendingJob { url, depth, target })
            .collect::<Vec<_>>();
        let (visited, fps) = {
            let v = self.visited.lock().unwrap_or_else(|p| p.into_inner());
            let f = self.seen_fp.lock().unwrap_or_else(|p| p.into_inner());
            (v.clone(), f.clone())
        };
        let mut ck = Checkpoint::new(&self.cfg.name, &self.cfg.hash());
        ck.global_pages = self.global_pages.load(Ordering::SeqCst);
        ck.pending = pending;
        for (i, t) in self.cfg.targets.iter().enumerate() {
            ck.targets.push(TargetState {
                id: t.id.clone(),
                visited: visited[i].iter().cloned().collect(),
                fp: fps[i].iter().map(|r| r.as_str().to_string()).collect(),
                written: self.per_target_count[i].load(Ordering::SeqCst),
            });
        }
        match ck.save(&self.path) {
            Ok(()) => {
                self.metrics.ckpt_written();
                tracing::debug!(
                    pending = ck.pending.len(),
                    visited = ck.targets.iter().map(|t| t.visited.len()).sum::<usize>(),
                    "checkpoint written"
                );
            }
            Err(e) => {
                // keep the delta so the next tick retries the snapshot
                self.pages_since.fetch_add(n + 1, Ordering::SeqCst);
                tracing::warn!(%e, path = %self.path.display(), "checkpoint write failed; will retry on next tick");
            }
        }
    }
}

fn load(a: &AtomicU64) -> u64 {
    a.load(Ordering::Relaxed)
}

/// Single construction point for crawl jobs: applies the run's queue-level
/// age policy (deadline) so every intake path (seeds, sitemap, DLQ replay,
/// discovery, resume) gets identical lifecycle semantics for free.
fn mk_job(cfg: &Config, url: Url, depth: usize, target: usize) -> Job {
    let j = Job::new(url, depth, target);
    if cfg.scheduling.job_max_age_secs > 0 {
        j.with_deadline(Duration::from_secs(cfg.scheduling.job_max_age_secs))
    } else {
        j
    }
}

/// Per-target extraction backend, chosen by `target.mode`.
enum ExtractMode {
    Html(Extractor),
    Json(JsonExtractor),
}

#[derive(Clone)]
struct WorkerCtx {
    worker_id: usize,
    cfg: Arc<Config>,
    fetcher: Arc<Fetcher>,
    politeness: Arc<Politeness>,
    pool: Arc<ProxyPool>,
    sink: Arc<dyn Sink>,
    metrics: Arc<Metrics>,
    queue: Arc<CrawlQueue>,
    runtime: Arc<Runtime>,
    supervisor: Arc<Supervisor>,
    /// The retry-window budget from the runtime (renamed from `budget` in
    /// the audit to keep "budget" meaning one thing: this is retry-window
    /// math; resource ceilings live in `crate::budget`).
    retry_budget: Option<Arc<RetryBudget>>,
    /// Run-scoped record cap (`max_records_per_run`; limit 0 = unlimited).
    record_cap: Arc<crate::budget::Budget>,
    cap_announced: Arc<std::sync::atomic::AtomicBool>,
    /// Per-job failure signal: `note_dlq` (the one place every failed
    /// exchange is recorded) sets it; `finish_job` consumes it. One worker
    /// owns one context at a time, so a plain flag is race-free by design.
    job_failed: Arc<AtomicBool>,
    job_fail_reason: Arc<std::sync::Mutex<String>>,
    visited: Arc<std::sync::Mutex<Vec<HashSet<String>>>>,
    seen_fp: Arc<std::sync::Mutex<Vec<HashSet<crate::ids::RecordId>>>>,
    per_target_count: Arc<Vec<AtomicUsize>>,
    outstanding: Arc<AtomicUsize>,
    global_pages: Arc<AtomicUsize>,
    extractors: Arc<Vec<ExtractMode>>,
    pipelines: Arc<Vec<TargetPipeline>>,
    robots_cache: RobotsMap,
    stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    per_target: Arc<std::sync::Mutex<std::collections::BTreeMap<String, TargetSummary>>>,
    dlq_fail: Arc<std::sync::Mutex<std::collections::BTreeMap<String, String>>>,
    dlq_ok: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    retried: Arc<std::collections::HashSet<String>>,
    ckpt: Option<Arc<CkptCtl>>,
    reval: Option<Arc<crate::reval::Store>>,
    audit: Option<Arc<crate::audit::Recorder>>,
}

async fn worker_loop(ctx: WorkerCtx) {
    loop {
        if ctx
            .stop
            .as_ref()
            .is_some_and(|s| s.load(std::sync::atomic::Ordering::Relaxed))
        {
            ctx.supervisor.note_draining(ctx.worker_id);
            tracing::debug!("stop flag set — draining");
            return;
        }
        match ctx.queue.pop() {
            Popped::Job(job) => {
                let (jid, tgt) = (job.id, job.target);
                let waited = job.submitted_at.elapsed();
                let url = job.url.to_string();
                ctx.runtime.acct.note_start();
                ctx.runtime.emit(
                    EventKind::JobStarted,
                    Some(jid.0),
                    json!({
                        "url": url,
                        "depth": job.depth,
                        "target": job.target,
                        "worker": ctx.worker_id,
                        "queue_wait_ms": waited.as_millis() as u64,
                    }),
                );
                ctx.supervisor.touch_running(ctx.worker_id);
                let res = ctx.process(job).await;
                ctx.queue.release(tgt);
                ctx.outstanding.fetch_sub(1, Ordering::SeqCst);
                ctx.finish_job(jid, tgt, &url, waited, res);
            }
            Popped::Expired(job) => {
                // Deadline-passed jobs are surfaced, counted, and retired —
                // never silently evicted. Visited marks stay: the URL was
                // seen; re-discovery in a later run may retry it.
                ctx.outstanding.fetch_sub(1, Ordering::SeqCst);
                ctx.runtime.acct.note_expired();
                ctx.runtime.emit(
                    EventKind::JobExpired,
                    Some(job.id.0),
                    json!({"url": job.url.as_str(), "target": job.target}),
                );
            }
            Popped::Empty => {
                if ctx.outstanding.load(Ordering::SeqCst) == 0 {
                    return;
                }
                ctx.supervisor.note_idle(ctx.worker_id);
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }
}

impl WorkerCtx {
    fn push(&self, job: Job) {
        // Intake count first: a worker must never conclude "nothing left"
        // while an enqueue is in flight (same ordering contract the bare
        // VecDeque queue had; the counter, not the mutex, carries it now).
        self.outstanding.fetch_add(1, Ordering::SeqCst);
        let (url, depth, target) = (job.url.to_string(), job.depth, job.target);
        match self.queue.push(job) {
            Ok(id) => {
                self.runtime.acct.note_submit();
                self.runtime.emit(
                    EventKind::JobSubmitted,
                    Some(id.0),
                    json!({"url": url, "depth": depth, "target": target}),
                );
            }
            Err(_) => {
                self.outstanding.fetch_sub(1, Ordering::SeqCst);
                self.metrics.queue_rejected();
                self.runtime
                    .emit(EventKind::QueueOverflow, None, json!({"dropped_url": url}));
            }
        }
    }

    /// One place that turns "a job left the pipeline" into consistent
    /// accounting: metric, run snapshot, and the JobCompleted/JobFailed
    /// events all agree because they all come from here.
    fn finish_job(&self, id: JobId, target: usize, url: &str, waited: Duration, res: Result<()>) {
        let policy_failed = self.job_failed.swap(false, Ordering::Relaxed);
        let reason = {
            let mut g = self
                .job_fail_reason
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            std::mem::take(&mut *g)
        };
        match (&res, policy_failed) {
            (Err(e), _) => {
                tracing::warn!(worker = self.worker_id, err = %e, "job failed");
                self.metrics.job_failed();
                self.supervisor.note_job_failed(self.worker_id);
                let why = if reason.is_empty() {
                    e.to_string()
                } else {
                    reason
                };
                self.runtime
                    .acct
                    .note_finish_failed(&crate::config::redact_credentials(url), &why);
                self.runtime.emit(
                    EventKind::JobFailed,
                    Some(id.0),
                    json!({"url": url, "target": target, "reason": why, "sink_error": true}),
                );
            }
            (Ok(()), true) => {
                self.metrics.job_failed();
                self.supervisor.note_job_failed(self.worker_id);
                let why = if reason.is_empty() {
                    "failed".to_string()
                } else {
                    reason
                };
                self.runtime
                    .acct
                    .note_finish_failed(&crate::config::redact_credentials(url), &why);
                self.runtime.emit(
                    EventKind::JobFailed,
                    Some(id.0),
                    json!({"url": url, "target": target, "reason": why, "sink_error": false}),
                );
            }
            (Ok(()), false) => {
                self.metrics.job_completed();
                self.supervisor.note_job_done(self.worker_id);
                self.runtime.acct.note_finish_ok(waited);
                self.runtime.emit(
                    EventKind::JobCompleted,
                    Some(id.0),
                    json!({"url": url, "target": target, "total_ms": waited.as_millis() as u64}),
                );
            }
        }
    }

    /// Feed the transport breaker + publish transition events. `ok` means
    /// "the server spoke" (any HTTP answer), not "the page was 200".
    fn breaker_note(&self, b: &Option<Arc<Breaker>>, ok: bool, host: &str) {
        let Some(b) = b else { return };
        let now = Instant::now();
        let ev = if ok {
            b.note_success(now)
        } else {
            b.note_failure(now)
        };
        self.brk_event(b, ev, host);
    }

    fn brk_event(&self, b: &Arc<Breaker>, ev: Option<BreakerEvent>, host: &str) {
        let Some(ev) = ev else { return };
        let kind = match ev {
            BreakerEvent::ClosedToOpen | BreakerEvent::HalfOpenToOpen => {
                Some(EventKind::BreakerOpened)
            }
            BreakerEvent::HalfOpenToClosed => Some(EventKind::BreakerClosed),
            _ => None,
        };
        if let Some(kind) = kind {
            let snap = b.snapshot();
            self.runtime.emit(
                kind,
                None,
                json!({
                    "host": host,
                    "state": snap.state,
                    "total_opens": snap.total_opens,
                    "transitions": snap.transitions,
                }),
            );
        }
    }

    /// Release a half-open probe slot when the job ended without a fetch
    /// verdict (robots-denied, dedupe, non-text after a probe was granted).
    fn breaker_release(&self, b: &Option<Arc<Breaker>>, took_probe: bool) {
        if took_probe {
            if let Some(b) = b {
                b.release_probe();
            }
        }
    }

    async fn process(&self, job: Job) -> Result<()> {
        let host = job.url.host_str().unwrap_or_default().to_string();
        if self.politeness.is_abandoned(&host, &self.cfg) {
            if let Some(a) = &self.audit {
                a.note_abandoned(&host);
            }
            return Ok(());
        }
        // Transport circuit breaker (per host). While a circuit is open the
        // job is retired to the failure path immediately — no socket is
        // opened, the upstream is left alone, and the reason is visible in
        // the DLQ line, the event stream and `GET /runtime`. This never
        // covers refusals (401/403/429/451): those are the politeness
        // layer's block-backoff domain, and mixing them would let a hostile
        // 403 wave silently switch the crawler off.
        let breaker = self.runtime.breaker_for(&host);
        let mut took_probe = false;
        if let Some(b) = &breaker {
            match b.permit(Instant::now()) {
                Permit::Open { retry_in } => {
                    self.metrics.breaker_rejected();
                    self.bump(job.target, |t| t.failed += 1);
                    self.note_dlq(
                        &job,
                        format!("breaker:open (retry in {}s)", retry_in.as_secs().max(1)),
                    );
                    tracing::debug!(host, ?retry_in, "transport breaker open; job skipped");
                    return Ok(());
                }
                Permit::Probe => took_probe = true,
                Permit::Allowed => {}
            }
        }
        let t_wait = Instant::now();
        self.politeness.wait(&host, &self.cfg, &self.metrics).await;
        if let Some(a) = &self.audit {
            a.note_wait(&host, t_wait.elapsed());
        }

        if self.cfg.respect_robots {
            let base = origin(&job.url);
            let robots = self.robots_for(&base).await;
            if let Some(d) = robots.crawl_delay {
                self.politeness.note_crawl_delay(&host, &self.cfg, d);
            }
            if !robots.allows(job.url.path()) {
                self.metrics.robot_denied();
                if let Some(a) = &self.audit {
                    a.note_robots_denial(&host);
                }
                if self.retried.contains(job.url.as_str()) {
                    self.dlq_fail
                        .lock()
                        .unwrap()
                        .insert(job.url.to_string(), "robots-denied".into());
                }
                tracing::debug!(url = %job.url, "denied by robots.txt");
                self.breaker_release(&breaker, took_probe);
                return Ok(());
            }
        }

        let rkey = dedup_key(&job.url);
        let hdrs = match self.reval.as_ref().map(|s| s.condition_for(&rkey)) {
            Some(crate::reval::Condition::Headers(h)) => h,
            _ => Vec::new(),
        };
        // Engine-path fetch: same transport and retry math as before, now
        // carrying the job's deadline, the shared retry budget and the
        // correlation id that ties every event of this exchange together.
        let mut plan = RequestPlan::new(job.url.clone());
        plan.headers = hdrs;
        plan.max_response_bytes = self.cfg.max_response_bytes;
        plan.deadline = job.deadline;
        let rctx = RequestContext {
            correlation_id: job.id.0,
            job_id: Some(job.id),
            target: job.target,
            host: host.clone(),
            bus: Some(self.runtime.bus.clone()),
            acct: Some(self.runtime.acct.clone()),
            budget: self.retry_budget.clone(),
        };
        let t_fetch = Instant::now();
        let fetch = self.fetcher.execute(plan, &rctx, &self.metrics).await;
        self.metrics
            .note_request_ms(t_fetch.elapsed().as_millis() as u64);
        let outcome = match fetch {
            Ok((o, _meta)) if o.status == 304 => {
                // Unchanged: no body, no parse, no record — but the cached
                // link graph replays discovery, so the crawl still advances.
                self.breaker_note(&breaker, true, &host);
                if let Some(a) = &self.audit {
                    a.note_request(&host, 304);
                }
                self.metrics.revalidated();
                self.bump(job.target, |t| t.revalidated += 1);
                if let Some(s) = &self.reval {
                    match s.snapshot(&rkey) {
                        Some(st) if !st.links.is_empty() => {
                            s.touch(&rkey);
                            if self.cfg.targets[job.target].follow_links
                                && job.depth < self.cfg.targets[job.target].max_depth
                            {
                                self.replay_links(st.links, &job).await;
                            }
                        }
                        Some(st) if st.etag.is_none() && st.lm.is_none() => s.drop_key(&rkey),
                        Some(_) => s.touch(&rkey),
                        None => {}
                    }
                }
                return Ok(());
            }
            Ok((o, _meta)) => {
                self.breaker_note(&breaker, true, &host);
                self.politeness.note_success(&host, &self.cfg);
                if let Some(a) = &self.audit {
                    a.note_request(&host, o.status);
                }
                o
            }
            Err((
                FetchError::Blocked {
                    status,
                    retry_after,
                },
                _meta,
            )) => {
                // A refusal is an answer: the transport works, the site
                // objected. The breaker stays closed (opening it on 403s
                // would let a hostile edge silently disable the crawler);
                // politeness handles refusals — that's its whole job.
                self.breaker_note(&breaker, true, &host);
                self.metrics.request_blocked();
                if let Some(a) = &self.audit {
                    a.note_request(&host, status);
                }
                self.bump(job.target, |t| t.blocked += 1);
                self.note_dlq(&job, format!("blocked:{status}"));
                let back = self.politeness.note_block(&host, &self.cfg, retry_after);
                self.metrics.note_backoff_ms(back.as_millis() as u64);
                if let Some(a) = &self.audit {
                    a.note_backoff(&host, back.as_millis() as u64);
                }
                tracing::warn!(
                    url = %job.url,
                    status,
                    backoff_s = back.as_secs(),
                    "access refused; honoring it (no spoofing, no evasion rotation)"
                );
                if self.pool.snapshot().len() > 1 {
                    tracing::info!(host, "note: pool failover exists for *proxy quality*, not for retrying refusals through other exits");
                }
                return Ok(());
            }
            Err((FetchError::Failed(msg), meta)) => {
                // Exhausted transport/5xx retries trip the breaker; a
                // deadline we imposed ourselves is not the host's fault.
                self.breaker_note(&breaker, meta.deadline_hit, &host);
                self.request_failed(&job, &msg);
                return Ok(());
            }
            Err((FetchError::Permanent(msg), _meta)) => {
                // Local cap (body size / bad proxy url): the upstream never
                // misbehaved, so the breaker stays uninvolved.
                self.breaker_note(&breaker, true, &host);
                self.request_failed(&job, &msg);
                return Ok(());
            }
            Err((FetchError::RobotsDenied, _meta)) => {
                self.breaker_release(&breaker, took_probe);
                return Ok(());
            }
        };

        let is_text =
            outcome.content_type.contains("html") || outcome.content_type.starts_with("text/");
        let is_json = matches!(self.extractors[job.target], ExtractMode::Json(_))
            && outcome.content_type.contains("json");
        if !is_text && !is_json {
            tracing::debug!(url = %job.url, ct = %outcome.content_type, "skipping non-text document");
            return Ok(());
        }

        // Typed fingerprint: the dedupe set stores validated ids, so a
        // corrupted hash can never silently split an entry in two.
        let fp = crate::ids::RecordId::parse(&extract::fingerprint(&outcome.body))
            .expect("fingerprint is a sha256 hex digest by construction");
        if !self.seen_fp.lock().unwrap_or_else(|p| p.into_inner())[job.target].insert(fp.clone()) {
            self.metrics.record_deduped();
            return Ok(());
        }

        let tcfg = &self.cfg.targets[job.target];
        let want_discover = tcfg.follow_links && job.depth < tcfg.max_depth;

        // Sync-only stage: `scraper::Html`/`Selector` are not Send, so parse,
        // extract, and collect hrefs here and hand only owned values forward.
        let (rec, hrefs, quarantined) = {
            let mut hrefs = Vec::new();
            let mut fields = match &self.extractors[job.target] {
                ExtractMode::Json(jx) => {
                    let Ok(doc) = serde_json::from_str::<serde_json::Value>(&outcome.body) else {
                        tracing::warn!(url = %job.url, "json-mode body is not valid JSON; record skipped");
                        return Ok(());
                    };
                    jx.extract(&doc)
                }
                ExtractMode::Html(ex) => {
                    let doc = Html::parse_document(&outcome.body);
                    let f = ex.extract(&doc);
                    if want_discover {
                        let sel: Selector = tcfg
                            .link_selector
                            .as_deref()
                            .and_then(|s| Selector::parse(s).ok())
                            .unwrap_or_else(|| {
                                Selector::parse("a[href]").expect("validated at startup")
                            });
                        hrefs = extract::links(&doc, &sel);
                    }
                    f
                }
            };
            // Transform pipeline, then declared-schema enforcement, on the
            // way to the sink. Identity pipelines (the default) cost one
            // branch; records that quarantine never reach the sink, but
            // their links are still discovered — the page was fetched
            // legitimately and its graph edges are sound data.
            let po = self.pipelines[job.target].apply(&mut fields, job.url.as_str(), job.depth)?;
            for _ in 0..po.drift {
                self.metrics.schema_drift();
            }
            if po.failures > 0 {
                self.metrics.pipeline_failures(po.failures);
            }
            let quarantined = po.quarantined;
            let rec = Record {
                run: self.cfg.name.clone(),
                target: self.cfg.targets[job.target].id.clone(),
                url: job.url.to_string(),
                depth: job.depth,
                ts: now_secs(),
                fingerprint: fp.as_str().to_string(),
                egress: outcome.proxy_id.clone(),
                fields,
            };
            (rec, hrefs, quarantined)
        };

        self.metrics.record_extracted();
        self.runtime.emit(
            EventKind::ExtractionCompleted,
            Some(job.id.0),
            json!({
                "url": job.url.as_str(),
                "target": rec.target,
                "fields": rec.fields.len(),
                "bytes": outcome.bytes,
            }),
        );
        if let Some(reasons) = &quarantined {
            self.metrics.record_quarantined();
            tracing::warn!(
                url = %job.url,
                target = %rec.target,
                "record quarantined by schema enforcement: {}",
                reasons.join("; ")
            );
            self.runtime.emit(
                EventKind::RecordQuarantined,
                Some(job.id.0),
                json!({
                    "url": job.url.as_str(),
                    "target": rec.target,
                    "reasons": reasons,
                    "fingerprint": &rec.fingerprint,
                }),
            );
            self.bump(job.target, |t| t.quarantined += 1);
        } else if !self.record_cap.try_take(1) {
            // `max_records_per_run` tripped: count, announce once (event +
            // log), flip the shared stop flag so the scheduler drains. The
            // record itself is skipped — never half-written, never an
            // error: a capped run is a graceful, successful run.
            self.metrics
                .note_budget_denied(crate::metrics::BudgetScope::Records);
            if !self.cap_announced.swap(true, Ordering::Relaxed) {
                tracing::warn!(
                    limit = cfg_limited(self.cfg.max_records_per_run),
                    "record budget exhausted — draining remaining work"
                );
                self.runtime.emit(
                    EventKind::BudgetExhausted,
                    None,
                    json!({
                        "budget": "records_per_run",
                        "limit": self.cfg.max_records_per_run,
                        "written": self.record_cap.used(),
                    }),
                );
                if let Some(s) = &self.stop {
                    s.store(true, Ordering::Relaxed);
                }
            }
        } else {
            let t_write = Instant::now();
            let wr = self.sink.write(&rec);
            self.metrics
                .note_storage_write(t_write.elapsed().as_millis() as u64);
            wr?;
            self.metrics.record_written();
            if let Some(t) = self.cfg.targets.get(job.target) {
                self.metrics.note_target_record(&t.id);
            }
            self.runtime.emit(
                EventKind::SinkWriteCompleted,
                Some(job.id.0),
                json!({"url": job.url.as_str(), "fingerprint": &rec.fingerprint}),
            );
            if let Some(a) = &self.audit {
                a.note_written(&host);
            }
            self.bump(job.target, |t| t.written += 1);
        }
        if self.retried.contains(job.url.as_str()) {
            self.dlq_ok
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(job.url.to_string());
        }

        let n = self.global_pages.fetch_add(1, Ordering::SeqCst) + 1;
        self.per_target_count[job.target].fetch_add(1, Ordering::SeqCst);
        if let Some(c) = &self.ckpt {
            c.pages_since.fetch_add(1, Ordering::SeqCst);
            c.tick(false);
        }

        // Absolutize + filter hrefs once; the filtered set feeds both the
        // queue and the revalidation cache (so a future 304 can replay it).
        let cands: Vec<Url> = if want_discover {
            hrefs
                .into_iter()
                .filter_map(|href| job.url.join(href.trim()).ok())
                .filter(|abs| {
                    abs.scheme() == job.url.scheme()
                        && abs.host_str() == job.url.host_str()
                        && abs.port() == job.url.port()
                        && (tcfg.include.is_empty()
                            || tcfg
                                .include
                                .iter()
                                .any(|p| abs.as_str().contains(p.as_str())))
                        && !tcfg
                            .exclude
                            .iter()
                            .any(|p| abs.as_str().contains(p.as_str()))
                })
                .collect()
        } else {
            Vec::new()
        };
        if let Some(s) = &self.reval {
            if outcome.status == 200 {
                let links: Vec<String> = if want_discover {
                    cands.iter().map(|u| u.to_string()).collect()
                } else {
                    Vec::new()
                };
                s.note_200(
                    &rkey,
                    outcome.etag.clone(),
                    outcome.last_modified.clone(),
                    links,
                );
            }
        }
        let within_budget = n < self.cfg.max_pages
            && self.per_target_count[job.target].load(Ordering::SeqCst)
                + self.queued_for(job.target)
                < tcfg.max_pages;
        if want_discover && within_budget {
            self.discover_links(cands, &job).await;
        }
        Ok(())
    }

    async fn discover_links(&self, cands: Vec<Url>, job: &Job) {
        for abs in cands {
            if !self.visited.lock().unwrap_or_else(|p| p.into_inner())[job.target]
                .insert(dedup_key(&abs))
            {
                continue;
            }
            if self.cfg.respect_robots {
                let robots = self.robots_for(&origin(&abs)).await;
                if !robots.allows(abs.path()) {
                    self.metrics.robot_denied();
                    if let Some(a) = &self.audit {
                        a.note_robots_denial(job.url.host_str().unwrap_or_default());
                    }
                    continue;
                }
            }
            self.push(mk_job(&self.cfg, abs, job.depth + 1, job.target));
        }
    }

    /// Discovery from a cached link set after a 304 (already absolute and
    /// filtered when captured; re-checked against robots because the site's
    /// policy may have changed even if the page didn't).
    async fn replay_links(&self, urls: Vec<String>, job: &Job) {
        for u in urls {
            let Ok(abs) = Url::parse(&u) else { continue };
            if !self.visited.lock().unwrap_or_else(|p| p.into_inner())[job.target]
                .insert(dedup_key(&abs))
            {
                continue;
            }
            if self.cfg.respect_robots {
                let robots = self.robots_for(&origin(&abs)).await;
                if !robots.allows(abs.path()) {
                    self.metrics.robot_denied();
                    if let Some(a) = &self.audit {
                        a.note_robots_denial(job.url.host_str().unwrap_or_default());
                    }
                    continue;
                }
            }
            self.push(mk_job(&self.cfg, abs, job.depth + 1, job.target));
        }
    }

    /// Fetch-and-cache robots.txt per origin (TTL from config).
    async fn robots_for(&self, base: &Url) -> Arc<Robots> {
        robots_for(
            &self.cfg,
            &self.fetcher,
            &self.metrics,
            &self.robots_cache,
            self.audit.as_deref(),
            base,
        )
        .await
    }

    fn bump(&self, target: usize, f: impl FnOnce(&mut TargetSummary)) {
        let id = self.cfg.targets[target].id.clone();
        let mut g = self.per_target.lock().unwrap_or_else(|p| p.into_inner());
        f(g.entry(id).or_default());
    }

    /// Uniform bookkeeping for "this exchange is over and it went badly":
    /// audit, per-target tally, DLQ record and the operator-visible warn.
    fn request_failed(&self, job: &Job, msg: &str) {
        if let Some(a) = &self.audit {
            a.note_error(job.url.host_str().unwrap_or_default());
        }
        self.bump(job.target, |t| t.failed += 1);
        self.note_dlq(job, format!("failed:{msg}"));
        tracing::warn!(url = %job.url, msg, "request failed after retries");
    }

    fn note_dlq(&self, job: &Job, reason: String) {
        // The failure signal for lifecycle accounting is recorded even when
        // no DLQ file is configured; only the persistence is gated.
        self.job_failed.store(true, Ordering::Relaxed);
        *self
            .job_fail_reason
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = reason.clone();
        if self.cfg.dlq_path.is_some() {
            self.dlq_fail
                .lock()
                .unwrap()
                .insert(job.url.to_string(), reason);
        }
    }

    /// Approximate per-target queue accounting (queue is small by design).
    fn queued_for(&self, target: usize) -> usize {
        self.queue.count_target(target)
    }
}

fn origin(u: &Url) -> Url {
    Url::parse(&format!("{}://{}", u.scheme(), u.authority())).unwrap_or_else(|_| u.clone())
}

fn dedup_key(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    let q = u.query().map(|s| format!("?{s}")).unwrap_or_default();
    format!("{host}{}{q}", u.path())
}

pub fn ua_token(ua: &str) -> String {
    ua.split(['/', ' ', '('])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("harvest")
        .to_lowercase()
}

/// Human rendering of a possibly-unlimited cap for logs.
fn cfg_limited(n: u64) -> String {
    if n == 0 {
        "unlimited".into()
    } else {
        n.to_string()
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
