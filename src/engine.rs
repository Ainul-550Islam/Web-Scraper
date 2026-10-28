//! The orchestrator: bounded worker pool over a shared job queue, with
//! robots enforcement, per-domain pacing, fingerprint dedup, sink writes,
//! and an embedded admin/metrics HTTP server.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::json;
use url::Url;

use crate::admin;
use crate::breaker::{Breaker, BreakerEvent, Permit};
#[cfg(feature = "browser")]
use crate::browser::{
    credentials::EnvironmentCredentialResolver, interaction, navigation, navigation::WaitUntil,
    BrowserRuntime,
};
use crate::config::Config;
use crate::events::EventKind;
use crate::exec::{ExecutionMeta, RequestContext, RequestPlan};
use crate::extract::{self, Extractor, JsonExtractor};
use crate::fetcher::{FetchError, Fetcher};
use crate::job::{Job, JobId};
use crate::metrics::Metrics;
use crate::policy::Politeness;
use crate::proxy::ProxyPool;
use crate::retry::RetryBudget;
use crate::robots::Robots;
use crate::runtime::{Runtime, RuntimeSlot};
use crate::sched::{CrawlQueue, Popped, PushError};
use crate::sink::{self, Record, Sink};
use crate::sitemap::{self, Sitemap};
use crate::supervisor::{CancelToken, RestartPolicy, Supervisor, TaskSet};
use crate::transform::TargetPipeline;
use scraper::{Html, Selector};

const MAX_DLQ_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DLQ_LINE_BYTES: usize = 1024 * 1024;
const MAX_DLQ_ENTRIES: usize = 100_000;

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

#[cfg(feature = "browser")]
struct BrowserFetch {
    outcome: crate::fetcher::FetchOutcome,
    challenge: crate::browser::challenge::ChallengeObservation,
    session_created: bool,
    selector_count: usize,
    action_count: usize,
}

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
    let drain_task = state
        .control()
        .and_then(|control| stop.clone().map(|flag| control.drain_on_stop(flag)));
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
    if let Some(control) = state.control() {
        control.drain().await;
    }
    if let Some(t) = admin_task {
        t.abort();
    }
    if let Some(t) = drain_task {
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
/// `stop`: workers stop admitting new jobs; an in-flight request interrupted
/// by operator cancellation is returned to the pending queue (and checkpointed
/// when configured), then workers exit. Queued-but-unstarted work is likewise
/// left pending rather than being discarded by the worker loop.
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
    let _output_guard = ctx.runtime.output_guard(stop.as_deref()).await?;
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
    let halt = Arc::new(AtomicBool::new(false));
    let fatal_error = Arc::new(std::sync::Mutex::new(None::<String>));
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
    let politeness = ctx.runtime.shared_pacing(&cfg);
    politeness.begin_epoch();
    let fetcher = Arc::new(
        Fetcher::new(cfg.clone(), pool.clone()).with_cancellation(
            stop.iter()
                .cloned()
                .chain(std::iter::once(halt.clone()))
                .collect(),
            Some(politeness.clone()),
        ),
    );
    let (sink, sink_path) = sink::build_for_targets(&cfg.output, &cfg.targets)?;
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
            .map(|t| {
                if t.selector_tree.is_some() {
                    Ok(if t.mode == "json" {
                        ExtractMode::TreeJson
                    } else {
                        ExtractMode::TreeHtml
                    })
                } else {
                    match t.mode.as_str() {
                        "json" => Ok(ExtractMode::Json(JsonExtractor::compile(&t.extract)?)),
                        _ => Ok(ExtractMode::Html(Extractor::compile(&t.extract)?)),
                    }
                }
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
        match crate::fsutil::remove_file_nofollow(p) {
            Ok(()) => tracing::info!(path = %p.display(), "fresh run — removed stale checkpoint"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("remove stale checkpoint {}", p.display()));
            }
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
    if let Some(path) = &cfg.dlq_path {
        let contents = match read_dlq(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).with_context(|| format!("read DLQ {}", path.display()));
            }
        };
        for (line_no, line) in contents.lines().enumerate() {
            if line.len() > MAX_DLQ_LINE_BYTES {
                anyhow::bail!(
                    "DLQ {} line {} exceeds the {}-byte limit",
                    path.display(),
                    line_no + 1,
                    MAX_DLQ_LINE_BYTES
                );
            }
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(line) else {
                tracing::warn!(line = line_no + 1, "ignoring malformed DLQ line");
                continue;
            };
            let Some(url) = value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
            else {
                tracing::warn!(line = line_no + 1, "ignoring DLQ entry without a URL");
                continue;
            };
            if dlq_existing.len() >= MAX_DLQ_ENTRIES {
                anyhow::bail!(
                    "DLQ {} exceeds the {}-entry limit",
                    path.display(),
                    MAX_DLQ_ENTRIES
                );
            }
            if value
                .get("attempts")
                .and_then(serde_json::Value::as_u64)
                .is_some_and(|attempts| attempts >= cfg.dlq_max_attempts as u64)
            {
                value["dead_for_good"] = serde_json::Value::Bool(true);
            }
            let key = url.to_string();
            if retry_dlq
                && value.get("retryable").and_then(serde_json::Value::as_bool) != Some(false)
                && value
                    .get("dead_for_good")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
            {
                retried.insert(key.clone());
            }
            dlq_existing.insert(key, value);
        }
    }

    // Sitemap-driven seeds (fetched before workers start, under the same
    // politeness + robots guarantees as regular crawl traffic).
    let seed_jobs: Vec<(usize, Url)> = if !resuming && cfg.targets.iter().any(|t| t.sitemap) {
        let sctx = SeedCtx {
            cfg: &cfg,
            fetcher: &fetcher,
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
            for url in &retried {
                let Ok(parsed) = Url::parse(url) else {
                    continue;
                };
                let Some(entry) = dlq_existing.get(url) else {
                    continue;
                };
                let target_index = target_index_for_dlq(&cfg, entry, &parsed);
                let Some(target_index) = target_index else {
                    continue;
                };
                if !crate::config::approved_url(&cfg.targets[target_index], &parsed, 0) {
                    continue;
                }
                visited.lock().unwrap_or_else(|p| p.into_inner())[target_index]
                    .insert(dedup_key(&parsed));
                if queue.push(mk_job(&cfg, parsed, 0, target_index)).is_ok() {
                    outstanding.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
    }

    #[cfg(feature = "browser")]
    let browser_runtime = if cfg.browser.enabled {
        let mut browser_cfg = cfg.browser.clone();
        if browser_cfg.user_agent_override.is_none() {
            browser_cfg.user_agent_override = Some(cfg.user_agent.clone());
        }
        Some(
            BrowserRuntime::launch(browser_cfg)
                .await
                .map_err(anyhow::Error::msg)?,
        )
    } else {
        None
    };

    let n_workers = cfg.concurrency.clamp(1, 64);
    let mut handles = Vec::new();
    for worker in 0..n_workers {
        let ctx = WorkerCtx {
            worker_id: worker,
            cfg: cfg.clone(),
            fetcher: fetcher.clone(),
            #[cfg(feature = "browser")]
            browser_runtime: browser_runtime.clone(),
            #[cfg(feature = "browser")]
            browser_egress: Arc::new(std::sync::Mutex::new(HashMap::new())),
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
            job_deferred: Arc::new(AtomicBool::new(false)),
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
            halt: halt.clone(),
            fatal_error: fatal_error.clone(),
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
    if let Some(message) = fatal_error.lock().unwrap_or_else(|p| p.into_inner()).take() {
        first_err.get_or_insert_with(|| anyhow::anyhow!(message));
    }
    if let Err(e) = sink::finish_owned(&sink).await {
        first_err.get_or_insert(e);
    }

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
            match crate::fsutil::remove_file_nofollow(&c.path) {
                Ok(()) => {
                    tracing::info!("run complete — checkpoint removed (nothing left to resume)")
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    tracing::info!("run complete — no checkpoint to remove");
                }
                Err(error) => {
                    first_err.get_or_insert_with(|| {
                        anyhow::anyhow!("checkpoint removal failed: {error}")
                    });
                }
            }
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
    if let Some(path) = &cfg.dlq_path {
        let ok = dlq_ok.lock().unwrap_or_else(|p| p.into_inner());
        let fails = dlq_fail.lock().unwrap_or_else(|p| p.into_inner());
        let mut kept: Vec<serde_json::Value> = Vec::new();
        let mut seen_urls: std::collections::HashSet<String> = Default::default();
        for (url, value) in &dlq_existing {
            seen_urls.insert(url.clone());
            if retry_dlq && ok.contains(url) {
                continue; // recovered — drop from DLQ
            }
            let mut next = value.clone();
            if retry_dlq
                && value
                    .get("dead_for_good")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true)
            {
                if let Some(reason) = fails.get(url) {
                    let attempts = value
                        .get("attempts")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0)
                        .saturating_add(1);
                    next["attempts"] = attempts.into();
                    next["reason"] = crate::config::redact_observation_text(reason).into();
                    if attempts >= cfg.dlq_max_attempts as u64 {
                        next["dead_for_good"] = serde_json::Value::Bool(true);
                    }
                }
            }
            kept.push(safe_dlq_entry(&cfg, url, next));
        }
        for (url, reason) in fails.iter() {
            if seen_urls.contains(url) || cfg.dlq_max_attempts < 1 {
                continue;
            }
            let target_id = target_id_for_url(&cfg, url);
            let mut entry = serde_json::json!({
                "url": url,
                "reason": crate::config::redact_observation_text(reason),
                "attempts": 1,
            });
            if let Some(target_id) = target_id {
                entry["target"] = target_id.into();
            }
            if cfg.dlq_max_attempts <= 1 {
                entry["dead_for_good"] = serde_json::Value::Bool(true);
            }
            kept.push(safe_dlq_entry(&cfg, url, entry));
        }
        match serialize_dlq_entries(&kept) {
            Ok(body) => match crate::fsutil::write_private_atomic(path, body.as_bytes()) {
                Ok(()) => {
                    tracing::info!(path = %path.display(), entries = kept.len(), "dlq rewritten")
                }
                Err(error) => {
                    first_err.get_or_insert_with(|| {
                        anyhow::anyhow!("DLQ write failed for {}: {error}", path.display())
                    });
                }
            },
            Err(error) => {
                first_err.get_or_insert(error.context("cannot safely serialize DLQ"));
            }
        }
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
    #[cfg(feature = "browser")]
    if let Some(runtime) = browser_runtime {
        if let Err(error) = runtime.shutdown().await {
            first_err
                .get_or_insert_with(|| anyhow::anyhow!("browser runtime cleanup failed: {error}"));
        }
    }
    tracing::info!(sink = %sink_path.display(), summary = ?summary, "run complete");
    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(summary)
}

fn read_dlq(path: &Path) -> std::io::Result<String> {
    let bytes = crate::fsutil::read_bounded(path, MAX_DLQ_BYTES)?;
    String::from_utf8(bytes)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
}

fn target_id_for_url(cfg: &Config, url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    cfg.targets
        .iter()
        .find(|target| crate::config::approved_url(target, &parsed, 0))
        .map(|target| target.id.clone())
}

fn target_index_for_dlq(cfg: &Config, entry: &serde_json::Value, url: &Url) -> Option<usize> {
    let recorded = entry
        .get("target_id")
        .and_then(serde_json::Value::as_str)
        .or_else(|| entry.get("target").and_then(serde_json::Value::as_str));
    if let Some(target_id) = recorded {
        if let Some(index) = cfg.targets.iter().position(|target| target.id == target_id) {
            return Some(index);
        }
        // Older distributed DLQ entries stored the Redis queue prefix in
        // `target`, not a configured target id. Resolve those by URL, then
        // let the caller re-check the exact approved target scope.
    }
    cfg.targets
        .iter()
        .position(|target| crate::config::approved_url(target, url, 0))
}

fn safe_dlq_entry(
    cfg: &Config,
    original_url: &str,
    mut entry: serde_json::Value,
) -> serde_json::Value {
    crate::config::redact_observation(&mut entry);
    let safe_url = crate::config::redact_credentials(original_url);
    if let Some(fields) = entry.as_object_mut() {
        fields.insert("url".into(), safe_url.clone().into());
        if !fields.contains_key("target") {
            if let Some(target_id) = target_id_for_url(cfg, original_url) {
                fields.insert("target".into(), target_id.into());
            }
        }
        if safe_url != original_url {
            fields.insert("retryable".into(), serde_json::Value::Bool(false));
        }
        if let Some(serde_json::Value::String(reason)) = fields.get_mut("reason") {
            *reason = truncate_utf8(&crate::config::redact_observation_text(reason), 4096);
        }
    }
    entry
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn serialize_dlq_entries(entries: &[serde_json::Value]) -> Result<String> {
    if entries.len() > MAX_DLQ_ENTRIES {
        anyhow::bail!("DLQ exceeds the {MAX_DLQ_ENTRIES}-entry limit");
    }
    let mut body = String::new();
    for entry in entries {
        let line = serde_json::to_string(entry).context("serialize DLQ entry")?;
        let line_bytes = line.len().saturating_add(1);
        if line_bytes > MAX_DLQ_LINE_BYTES {
            anyhow::bail!("DLQ entry exceeds the {MAX_DLQ_LINE_BYTES}-byte line limit");
        }
        if (body.len() as u64).saturating_add(line_bytes as u64) > MAX_DLQ_BYTES {
            anyhow::bail!("DLQ exceeds the {MAX_DLQ_BYTES}-byte total limit");
        }
        body.push_str(&line);
        body.push('\n');
    }
    Ok(body)
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
    metrics: &'a Arc<Metrics>,
    robots_cache: &'a RobotsMap,
    audit: Option<Arc<crate::audit::Recorder>>,
}

impl SeedCtx<'_> {
    /// robots-gate + pace + fetch one sitemap document body.
    async fn grab(&self, u: &Url) -> Option<String> {
        let approved = self
            .cfg
            .targets
            .iter()
            .flat_map(|t| std::iter::once(t.start_url.as_str()).chain(t.sitemap_url.as_deref()))
            .filter_map(|s| Url::parse(s).ok())
            .any(|start| {
                start.origin() == u.origin()
                    && start.username() == u.username()
                    && start.password() == u.password()
            });
        if !approved || u.as_str().len() > 8192 {
            return None;
        }
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
            if !r.allows_url(u) {
                if let Some(a) = &self.audit {
                    a.note_robots_denial(&host);
                }
                tracing::warn!(url = %crate::config::redact_credentials(u.as_str()), "sitemap refused by robots.txt; honoring");
                return None;
            }
        }
        let t = Instant::now();
        if !self.fetcher.wait_initial(&host, self.metrics).await {
            return None;
        }
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
                tracing::warn!(url = %crate::config::redact_credentials(u.as_str()), err = ?e, "sitemap fetch failed; continuing without it");
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
        if out.len() >= MAX_TOTAL_URLS.min(cfg.max_pages) {
            break;
        }
        let candidate_cap = (MAX_TOTAL_URLS.min(cfg.max_pages) - out.len()).min(t.max_pages);
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
            Sitemap::Urls(v) => candidates.extend(
                v.iter()
                    .filter(|l| l.len() <= 8192)
                    .filter_map(|l| rel(&su, l))
                    .take(candidate_cap),
            ),
            Sitemap::Index(children) => {
                for c in children.iter().take(MAX_CHILD_DOCS) {
                    if candidates.len() >= candidate_cap {
                        break;
                    }
                    let Some(cu) = rel(&su, c) else { continue };
                    if let Some(cb) = ctx.grab(&cu).await {
                        if let Sitemap::Urls(inner) = sitemap::parse(&cb) {
                            let remaining = candidate_cap - candidates.len();
                            candidates.extend(
                                inner
                                    .iter()
                                    .filter(|l| l.len() <= 8192)
                                    .filter_map(|l| rel(&cu, l))
                                    .take(remaining),
                            );
                        } else {
                            tracing::warn!(url = %crate::config::redact_credentials(cu.as_str()), "nested sitemapindex ignored (depth 1 only)");
                        }
                    }
                }
            }
        }
        let mut pushed = 0usize;
        for abs in candidates {
            if out.len() >= MAX_TOTAL_URLS.min(cfg.max_pages) || pushed >= candidate_cap {
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
                if !r.allows_url(&abs) {
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
pub(crate) enum ExtractMode {
    Html(Extractor),
    Json(JsonExtractor),
    TreeHtml,
    TreeJson,
}

#[derive(Clone)]
struct WorkerCtx {
    worker_id: usize,
    cfg: Arc<Config>,
    fetcher: Arc<Fetcher>,
    #[cfg(feature = "browser")]
    browser_runtime: Option<BrowserRuntime>,
    #[cfg(feature = "browser")]
    browser_egress: Arc<std::sync::Mutex<HashMap<usize, usize>>>,

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
    /// A popped job was returned to the queue instead of reaching a terminal
    /// result; finish_job releases its active slot without marking success.
    job_deferred: Arc<AtomicBool>,
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
    halt: Arc<AtomicBool>,
    fatal_error: Arc<std::sync::Mutex<Option<String>>>,
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
        if ctx.halt.load(Ordering::Relaxed)
            || ctx
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
                if let Err(ref e) = res {
                    ctx.halt.store(true, Ordering::Relaxed);
                    ctx.bump(tgt, |t| t.failed += 1);
                    ctx.fatal_error
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .get_or_insert_with(|| e.to_string());
                }
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
    #[cfg(feature = "browser")]
    async fn fetch_browser(
        &self,
        job: &Job,
        target: &crate::config::TargetCfg,
    ) -> crate::browser::error::BrowserResult<BrowserFetch> {
        use crate::browser::config::SessionIsolation;
        use crate::browser::error::BrowserError;
        use crate::browser::session::BrowserSession;
        use crate::proxy::provider::ProxyEndpoint;

        let runtime = self
            .browser_runtime
            .as_ref()
            .ok_or(BrowserError::Unsupported(
                "configured browser runtime is absent",
            ))?;
        let session_key = match self.cfg.browser.session_isolation {
            SessionIsolation::PerJob => format!("crawl.{}.job.{}", target.id, job.id.0),
            SessionIsolation::PerTargetWorker => {
                format!("crawl.{}.worker.{}", target.id, self.worker_id)
            }
        };
        let proxy_index = if self.cfg.browser.session_isolation == SessionIsolation::PerTargetWorker
        {
            let mut choices = self
                .browser_egress
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            *choices.entry(job.target).or_insert_with(|| {
                self.pool
                    .pick(((job.target as u64) << 32) | self.worker_id as u64)
            })
        } else {
            self.pool.pick(job.id.0)
        };
        let egress_id = self.pool.id(proxy_index);
        let _hold = self.pool.try_hold(proxy_index).ok_or_else(|| {
            self.metrics
                .note_budget_denied(crate::metrics::BudgetScope::Proxy);
            BrowserError::Policy("configured proxy egress is at its in-flight cap".into())
        })?;
        let proxy_url = self.pool.proxy_url(proxy_index).flatten();
        let proxy_capability = if let Some(url) = proxy_url.as_deref() {
            let endpoint = ProxyEndpoint {
                id: egress_id.clone(),
                url: url.to_owned(),
                region: None,
                username_secret_ref: None,
                password_secret_ref: None,
                supports_session_affinity: false,
                supports_region_selection: false,
            };
            Some(crate::proxy::browser::capabilities(&endpoint)?)
        } else {
            None
        };
        let session_created = !runtime.contains_session(&session_key).await;
        let session: Arc<BrowserSession> = runtime
            .session(&session_key, Some(&egress_id), proxy_capability)
            .await?;
        let start_host = url::Url::parse(&target.start_url)
            .map_err(|_| BrowserError::InvalidInput("target start URL"))?
            .host_str()
            .ok_or(BrowserError::InvalidInput("target host"))?
            .to_ascii_lowercase();
        let mut allowed = std::collections::BTreeSet::new();
        allowed.insert(start_host);
        allowed.extend(
            self.cfg
                .browser
                .authorized_hosts
                .iter()
                .map(|h| h.to_ascii_lowercase()),
        );
        allowed.extend(
            target
                .browser
                .allowed_hosts
                .iter()
                .map(|h| h.to_ascii_lowercase()),
        );
        let allowed_hosts: Vec<String> = allowed.into_iter().collect();
        let request_host = job
            .url
            .host_str()
            .ok_or(BrowserError::InvalidInput("crawl URL host"))?
            .to_ascii_lowercase();
        if !allowed_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&request_host))
        {
            return Err(BrowserError::Policy(
                "browser target URL is outside the explicit host scope".into(),
            ));
        }
        let t0 = Instant::now();
        let result = async {
            let _navigation = navigation::navigate(
                &session,
                job.url.as_str(),
                WaitUntil::Load,
                self.cfg.browser.navigation_timeout_ms,
            )
            .await?;
            let resolver = EnvironmentCredentialResolver;
            let workflow = interaction::run_workflow_scoped(
                &session,
                &self.cfg.browser,
                &target.browser.workflow,
                &resolver,
                &allowed_hosts,
            )
            .await?;
            let page = runtime.observe(&session, job.url.as_str()).await?;
            let final_url = page
                .final_url
                .as_deref()
                .ok_or(BrowserError::Protocol { status: 502 })?;
            let final_parsed =
                url::Url::parse(final_url).map_err(|_| BrowserError::Protocol { status: 502 })?;
            let final_host = final_parsed
                .host_str()
                .ok_or(BrowserError::Protocol { status: 502 })?;
            if !allowed_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(final_host))
            {
                return Err(BrowserError::Policy(
                    "browser redirect ended outside the explicit host scope".into(),
                ));
            }
            if !final_parsed.username().is_empty() || final_parsed.password().is_some() {
                return Err(BrowserError::Policy(
                    "browser final URL contains credentials".into(),
                ));
            }
            let is_json=target.mode=="json"||page.content_type.as_deref().is_some_and(|ct|ct.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/json"));
            let body = if is_json {
                let value=session.execute("return document.body ? document.body.innerText : document.documentElement.innerText;",Vec::new(),self.cfg.browser.max_html_bytes).await?;
                value.as_str().map(ToOwned::to_owned).ok_or(BrowserError::NotFound("browser JSON body"))?
            }else{
                page.html.clone().ok_or(BrowserError::NotFound("browser HTML snapshot"))?
            };
            if body.len() as u64 > self.cfg.max_response_bytes {
                return Err(BrowserError::ContentLimit {
                    kind: "browser response",
                    limit: self.cfg.max_response_bytes as usize,
                });
            }
            let selector_count = if let Some(tree) = &target.selector_tree {
                let rows = if is_json { crate::selector::engine::extract_json(&body,tree,10_000)? } else { crate::selector::engine::extract(&body, tree, 10_000)? };
                rows.iter()
                    .map(|r| r.children.values().map(Vec::len).sum::<usize>())
                    .sum::<usize>()
                    .max(rows.len())
            } else {
                0
            };
            let challenge = crate::browser::challenge::detect(&page);
            let final_url =
                url::Url::parse(final_url).map_err(|_| BrowserError::Protocol { status: 502 })?;
            let elapsed = t0.elapsed().as_millis().min(u64::MAX as u128) as u64;
            let outcome = crate::fetcher::FetchOutcome {
                url: final_url,
                etag: None,
                last_modified: None,
                status: page.status.unwrap_or(200),
                content_type: page
                    .content_type
                    .clone()
                    .unwrap_or_else(|| "text/html; charset=utf-8".into()),
                bytes: body.len(),
                body,
                latency_ms: elapsed,
                proxy_id: egress_id.clone(),
            };
            Ok::<_, BrowserError>((
                outcome,
                challenge,
                selector_count,
                workflow.completed_steps,
                page,
            ))
        }
        .await;
        let elapsed = t0.elapsed().as_millis().min(u64::MAX as u128) as u64;
        self.pool.report(proxy_index, result.is_ok(), elapsed);
        if self.cfg.browser.session_isolation == SessionIsolation::PerJob {
            let _ = runtime.close_session(&session_key).await;
        }
        let (outcome, challenge, selector_count, completed_steps, _page) = result?;
        Ok(BrowserFetch {
            outcome,
            challenge,
            session_created,
            selector_count,
            action_count: completed_steps,
        })
    }

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
            Err(PushError::Full) => {
                self.outstanding.fetch_sub(1, Ordering::SeqCst);
                self.metrics.queue_rejected();
                self.runtime
                    .emit(EventKind::QueueOverflow, None, json!({"dropped_url": url}));
            }
            Err(PushError::InvalidTarget) => {
                self.outstanding.fetch_sub(1, Ordering::SeqCst);
                let message = format!(
                    "queue rejected job for target index {target}; configured target count is {}",
                    self.cfg.targets.len()
                );
                tracing::error!(
                    job_target = target,
                    url = %crate::config::redact_credentials(&url),
                    %message,
                    "invalid target index reached the crawl queue"
                );
                self.halt.store(true, Ordering::Relaxed);
                self.fatal_error
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get_or_insert(message);
            }
        }
    }

    fn requeue(&self, job: Job) -> Result<()> {
        self.outstanding.fetch_add(1, Ordering::SeqCst);
        let (url, depth, target) = (job.url.to_string(), job.depth, job.target);
        match self.queue.requeue(job) {
            Ok(id) => {
                self.runtime.acct.note_submit();
                self.runtime.emit(
                    EventKind::JobSubmitted,
                    Some(id.0),
                    json!({"url": url, "depth": depth, "target": target, "requeued": true}),
                );
                Ok(())
            }
            Err(error) => {
                self.outstanding.fetch_sub(1, Ordering::SeqCst);
                anyhow::bail!("could not preserve a cancelled job in the queue: {error:?}");
            }
        }
    }

    /// One place that turns "a job left the pipeline" into consistent
    /// accounting: metric, run snapshot, and the JobCompleted/JobFailed
    /// events all agree because they all come from here.
    fn finish_job(&self, id: JobId, target: usize, url: &str, waited: Duration, res: Result<()>) {
        if self.job_deferred.swap(false, Ordering::Relaxed) {
            self.runtime.acct.note_deferred();
            return;
        }
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
                tracing::warn!(
                    worker = self.worker_id,
                    err = %crate::config::redact_observation_text(&e.to_string()),
                    "job failed"
                );
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
        let approved = self.cfg.targets.get(job.target).ok_or_else(|| {
            anyhow::anyhow!(
                "job target index {} is out of range for {} configured targets",
                job.target,
                self.cfg.targets.len()
            )
        })?;
        let host = job.url.host_str().unwrap_or_default().to_string();
        if !crate::config::approved_url(approved, &job.url, job.depth) {
            self.note_dlq(&job, "scope: URL outside the approved target".into());
            return Ok(());
        }
        if self.global_pages.load(Ordering::SeqCst) >= self.cfg.max_pages
            || self.per_target_count[job.target].load(Ordering::SeqCst) >= approved.max_pages
        {
            return Ok(());
        }
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
        if self.cfg.respect_robots {
            let base = origin(&job.url);
            let robots = self.robots_for(&base).await;
            if let Some(d) = robots.crawl_delay {
                self.politeness.note_crawl_delay(&host, &self.cfg, d);
            }
            if !robots.allows_url(&job.url) {
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
                tracing::debug!(url = %crate::config::redact_credentials(job.url.as_str()), "denied by robots.txt");
                self.breaker_release(&breaker, took_probe);
                return Ok(());
            }
        }

        if self.politeness.is_abandoned(&host, &self.cfg) {
            self.breaker_release(&breaker, took_probe);
            return Ok(());
        }
        let t_wait = Instant::now();
        if !self
            .politeness
            .wait_stoppable(
                &host,
                &self.cfg,
                &self.metrics,
                self.stop.as_deref(),
                &self.halt,
            )
            .await
        {
            self.breaker_release(&breaker, took_probe);
            self.requeue(job)?; // the checkpoint retains not-yet-fetched work
            self.job_deferred.store(true, Ordering::Relaxed);
            return Ok(());
        }
        if let Some(a) = &self.audit {
            a.note_wait(&host, t_wait.elapsed());
        }

        // Hard page admission BEFORE the request. Queuing 1000 links can
        // never turn a max_pages=3 run into 1001 requests under fanout.
        let target_counter = &self.per_target_count[job.target];
        let target_cap = self.cfg.targets[job.target].max_pages;
        if target_counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < target_cap).then(|| n + 1)
            })
            .is_err()
        {
            self.breaker_release(&breaker, took_probe);
            return Ok(());
        }
        if self
            .global_pages
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < self.cfg.max_pages).then(|| n + 1)
            })
            .is_err()
        {
            target_counter.fetch_sub(1, Ordering::SeqCst);
            self.breaker_release(&breaker, took_probe);
            return Ok(());
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
        let fetch = if approved.browser.enabled {
            #[cfg(feature = "browser")]
            {
                self.metrics.request_started();
                self.runtime.emit(EventKind::RequestStarted, Some(job.id.0), json!({"url":crate::config::redact_credentials(job.url.as_str()),"transport":"browser","target":approved.id}));
                match self.fetch_browser(&job, approved).await {
                    Ok(browser_fetch) => {
                        let elapsed = t_fetch.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        let status = browser_fetch.outcome.status;
                        let meta = ExecutionMeta {
                            correlation_id: job.id.0,
                            attempts: 1,
                            total_ms: elapsed,
                            last_attempt_ms: elapsed,
                            bytes: browser_fetch.outcome.bytes as u64,
                            status,
                            egress: browser_fetch.outcome.proxy_id.clone(),
                            backoff_ms: 0,
                            deadline_hit: false,
                        };
                        self.runtime.acct.note_bytes(meta.bytes);
                        self.runtime.emit(EventKind::RequestFinished,Some(job.id.0),json!({"status":status,"bytes":meta.bytes,"egress":meta.egress,"transport":"browser"}));
                        if matches!(status, 401 | 403 | 429 | 451) {
                            Err((
                                FetchError::Blocked {
                                    status,
                                    retry_after: None,
                                },
                                meta,
                            ))
                        } else if browser_fetch.challenge.detected {
                            self.metrics.request_blocked();
                            self.metrics.browser_challenge_detected();
                            self.bump(job.target, |t| t.blocked += 1);
                            self.note_dlq(
                                &job,
                                format!("browser-challenge:{:?}", browser_fetch.challenge.kind),
                            );
                            self.breaker_note(&breaker, true, &host);
                            self.runtime.emit(EventKind::BrowserChallengeDetected,Some(job.id.0),json!({"target":approved.id,"kind":browser_fetch.challenge.kind,"confidence":browser_fetch.challenge.confidence,"evidence":browser_fetch.challenge.evidence,"url":crate::config::redact_credentials(job.url.as_str()),"decision":"stop"}));
                            if let Some(a) = &self.audit {
                                a.note_request(&host, status);
                            }
                            return Ok(());
                        } else {
                            self.metrics.request_ok();
                            self.metrics.page_fetched();
                            for _ in 0..browser_fetch.action_count {
                                self.metrics.browser_action_completed();
                            }
                            if browser_fetch.session_created {
                                self.metrics.browser_session_created();
                                self.runtime.emit(EventKind::BrowserSessionCreated,Some(job.id.0),json!({"target":approved.id,"egress":meta.egress,"worker":self.worker_id}));
                            }
                            if browser_fetch.selector_count > 0 {
                                self.metrics
                                    .selector_records_previewed(browser_fetch.selector_count);
                                self.runtime.emit(EventKind::SelectorPreviewCompleted,Some(job.id.0),json!({"target":approved.id,"records":browser_fetch.selector_count}));
                            }
                            Ok((browser_fetch.outcome, meta))
                        }
                    }
                    Err(error) => {
                        let elapsed = t_fetch.elapsed().as_millis().min(u64::MAX as u128) as u64;
                        let message = error.to_string();
                        self.runtime.emit(EventKind::RequestFinished,Some(job.id.0),json!({"status":0,"transport":"browser","error":crate::browser::error::safe_message(&message)}));
                        let failure = match &error {
                            crate::browser::error::BrowserError::Policy(_)
                            | crate::browser::error::BrowserError::InvalidInput(_)
                            | crate::browser::error::BrowserError::ContentLimit { .. }
                            | crate::browser::error::BrowserError::Unsupported(_)
                            | crate::browser::error::BrowserError::Config(_) => {
                                FetchError::Permanent(message)
                            }
                            _ => FetchError::Failed(message),
                        };
                        Err((
                            failure,
                            ExecutionMeta {
                                correlation_id: job.id.0,
                                attempts: 1,
                                total_ms: elapsed,
                                last_attempt_ms: elapsed,
                                bytes: 0,
                                status: 0,
                                egress: "browser".into(),
                                backoff_ms: 0,
                                deadline_hit: false,
                            },
                        ))
                    }
                }
            }
            #[cfg(not(feature = "browser"))]
            {
                Err((
                    FetchError::Permanent(
                        "browser target requires a binary built with --features browser".into(),
                    ),
                    ExecutionMeta::default(),
                ))
            }
        } else {
            self.fetcher.execute(plan, &rctx, &self.metrics).await
        };
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
                    url = %crate::config::redact_credentials(job.url.as_str()),
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
                if msg == "request cancelled"
                    && self
                        .stop
                        .as_ref()
                        .is_some_and(|stop| stop.load(Ordering::Relaxed))
                {
                    // Operator cancellation is not an upstream failure. Keep
                    // the admitted job pending for checkpoint/recovery instead
                    // of counting or persisting it as a permanent DLQ entry.
                    self.breaker_release(&breaker, took_probe);
                    self.requeue(job)?;
                    self.job_deferred.store(true, Ordering::Relaxed);
                    return Ok(());
                }
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
        let is_json = matches!(
            self.extractors[job.target],
            ExtractMode::Json(_) | ExtractMode::TreeJson
        ) && outcome.content_type.contains("json");
        if !is_text && !is_json {
            tracing::debug!(url = %crate::config::redact_credentials(job.url.as_str()), ct = %outcome.content_type, "skipping non-text document");
            return Ok(());
        }

        // Typed fingerprint: the dedupe set stores validated ids, so a
        // corrupted hash can never silently split an entry in two.
        if !is_json {
            extract::guard_html(&outcome.body)?;
        }
        let body_fp = if is_json {
            extract::fingerprint_json(&outcome.body)?
        } else {
            extract::fingerprint(&outcome.body)
        };
        let fp = crate::ids::RecordId::parse(&body_fp)
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
                        tracing::warn!(url = %crate::config::redact_credentials(job.url.as_str()), "json-mode body is not valid JSON; record skipped");
                        return Ok(());
                    };
                    jx.extract_bounded(&doc)?
                }
                ExtractMode::Html(ex) => {
                    let doc = Html::parse_document(&outcome.body);
                    let f = ex.extract_bounded(&doc)?;
                    if want_discover {
                        let sel: Selector = tcfg
                            .link_selector
                            .as_deref()
                            .and_then(|s| Selector::parse(s).ok())
                            .unwrap_or_else(|| {
                                Selector::parse("a[href]").expect("validated at startup")
                            });
                        hrefs = extract::links_bounded(
                            &doc,
                            &sel,
                            self.cfg.max_pages.min(tcfg.max_pages),
                        );
                    }
                    f
                }
                ExtractMode::TreeHtml => {
                    let tree = tcfg
                        .selector_tree
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("validated selector tree disappeared"))?;
                    let rows = crate::selector::engine::extract(&outcome.body, tree, 10_000)?;
                    if want_discover {
                        let doc = Html::parse_document(&outcome.body);
                        let sel: Selector = tcfg
                            .link_selector
                            .as_deref()
                            .and_then(|s| Selector::parse(s).ok())
                            .unwrap_or_else(|| {
                                Selector::parse("a[href]").expect("validated at startup")
                            });
                        hrefs = extract::links_bounded(
                            &doc,
                            &sel,
                            self.cfg.max_pages.min(tcfg.max_pages),
                        );
                    }
                    rows.first()
                        .map(|r| r.to_value().as_object().cloned().unwrap_or_default())
                        .unwrap_or_default()
                }
                ExtractMode::TreeJson => {
                    let tree = tcfg
                        .selector_tree
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("validated selector tree disappeared"))?;
                    let rows = crate::selector::engine::extract_json(&outcome.body, tree, 10_000)?;
                    rows.first()
                        .map(|r| r.to_value().as_object().cloned().unwrap_or_default())
                        .unwrap_or_default()
                }
            };
            // Transform pipeline, then declared-schema enforcement, on the
            // way to the sink. Identity pipelines (the default) cost one
            // branch; records that quarantine never reach the sink, but
            // their links are still discovered — the page was fetched
            // legitimately and its graph edges are sound data.
            let po =
                self.pipelines[job.target].apply(&mut fields, outcome.url.as_str(), job.depth)?;
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
                url: crate::config::redact_credentials(outcome.url.as_str()),
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
                url = %crate::config::redact_credentials(job.url.as_str()),
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
                self.halt.store(true, Ordering::Relaxed);
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

        let n = self.global_pages.load(Ordering::SeqCst);
        if let Some(c) = &self.ckpt {
            c.pages_since.fetch_add(1, Ordering::SeqCst);
            c.tick(false);
        }

        // Absolutize + filter hrefs once; the filtered set feeds both the
        // queue and the revalidation cache (so a future 304 can replay it).
        let cands: Vec<Url> = if want_discover {
            hrefs
                .into_iter()
                .filter(|href| href.len() <= 8192)
                .filter_map(|href| outcome.url.join(href.trim()).ok())
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
                .take(self.cfg.max_pages.min(tcfg.max_pages))
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
            if self
                .global_pages
                .load(Ordering::SeqCst)
                .saturating_add(self.queue.len())
                >= self.cfg.max_pages
                || self.per_target_count[job.target]
                    .load(Ordering::SeqCst)
                    .saturating_add(self.queued_for(job.target))
                    >= self.cfg.targets[job.target].max_pages
            {
                break;
            }
            if !crate::config::approved_url(&self.cfg.targets[job.target], &abs, job.depth + 1) {
                continue;
            }
            if !self.visited.lock().unwrap_or_else(|p| p.into_inner())[job.target]
                .insert(dedup_key(&abs))
            {
                continue;
            }
            if self.cfg.respect_robots {
                let robots = self.robots_for(&origin(&abs)).await;
                if !robots.allows_url(&abs) {
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
            if self
                .global_pages
                .load(Ordering::SeqCst)
                .saturating_add(self.queue.len())
                >= self.cfg.max_pages
                || self.per_target_count[job.target]
                    .load(Ordering::SeqCst)
                    .saturating_add(self.queued_for(job.target))
                    >= self.cfg.targets[job.target].max_pages
            {
                break;
            }
            let Ok(abs) = Url::parse(&u) else { continue };
            if !crate::config::approved_url(&self.cfg.targets[job.target], &abs, job.depth + 1) {
                continue;
            }
            if !self.visited.lock().unwrap_or_else(|p| p.into_inner())[job.target]
                .insert(dedup_key(&abs))
            {
                continue;
            }
            if self.cfg.respect_robots {
                let robots = self.robots_for(&origin(&abs)).await;
                if !robots.allows_url(&abs) {
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
        let Some(id) = self.cfg.targets.get(target).map(|t| t.id.clone()) else {
            tracing::error!(
                job_target = target,
                configured_targets = self.cfg.targets.len(),
                "cannot update per-target summary for an invalid target index"
            );
            return;
        };
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
        let safe_message = crate::config::redact_observation_text(msg);
        self.note_dlq(job, format!("failed:{safe_message}"));
        tracing::warn!(
            url = %crate::config::redact_credentials(job.url.as_str()),
            msg = %safe_message,
            "request failed after retries"
        );
    }

    fn note_dlq(&self, job: &Job, reason: String) {
        // The failure signal for lifecycle accounting is recorded even when
        // no DLQ file is configured; only the persistence is gated.
        let reason = crate::config::redact_observation_text(&reason);
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
