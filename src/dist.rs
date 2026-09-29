//! Distributed mode: a Redis-backed crawl queue so many `harvest worker`
//! processes (or shards) drain one job pool cooperatively.
//!
//! Semantics:
//! * `push` seeds each target's start URL into `{prefix}:jobs`.
//! * Workers pop with `BRPOPLPUSH jobs -> inflight` (a *reliable queue*: an
//!   item stays visible in `inflight` until acked, so a crashed worker never
//!   silently loses a job; `flush_prefix`/startup sweeps can return strays).
//! * Cross-process dedup: enqueue-dedup and content fingerprints live in
//!   Redis sets, so two shards can race on the same host without writing the
//!   same page twice.
//! * Politeness is *shared*: the per-host fixed-window limiter counts
//!   requests across every worker, so N workers do not multiply the agreed
//!   request rate. robots.txt is fetched once and cached in Redis with a TTL.
//! * A running job holds a short-TTL *lease* heartbeat (`lease:<digest>`)
//!   naming its holder (`<worker>-s<seq>-<hex>`); `sweep_inflight` — the
//!   startup stray-reclaimer — skips anything whose lease is still fresh, so
//!   restarting the cohort cannot double-process slow-but-alive work.
//! * Failures re-enter the queue up to `max_retries` (queue-level retry —
//!   the HTTP fetcher itself runs with retries disabled); refusals and
//!   exhausted jobs land on `{prefix}:dlq` as JSON lines with a reason.
//! * 401/403/451 are *not* retried here: a refusal is a policy answer, not
//!   a transient fault, and retrying it would be the evasion the project's
//!   compliance policy forbids.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use redis::AsyncCommands;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use url::Url;

use sha2::Digest;

use crate::breaker::Permit;
use crate::config::Config;
use crate::engine::ua_token;
use crate::events::EventKind;
use crate::exec::{RequestContext, RequestPlan};
use crate::extract::{self, Extractor};
use crate::fetcher::{FetchError, Fetcher};
use crate::ids::{is_safe_key_token, LeaseId, WorkerId};
use crate::metrics::Metrics;
use crate::proxy::ProxyPool;
use crate::robots::Robots;
use crate::runtime::Runtime;
use crate::sink::{self, Record};

/// Queue-level job age policy: `Some(unix-seconds)` when the config caps how
/// long queued work stays fresh (`scheduling.job_max_age_secs`; 0 disables,
/// mirroring the engine's default-off stance for every scheduling cap).
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn expire_at(cfg: &Config) -> Option<u64> {
    (cfg.scheduling.job_max_age_secs > 0).then(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|x| x.as_secs() + cfg.scheduling.job_max_age_secs)
            .unwrap_or(0)
    })
}

/// One unit of crawl work. Kept as compact JSON so it is human-inspectable
/// with `redis-cli LRange <prefix>:jobs 0 -1`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub url: String,
    pub depth: usize,
    pub target: usize,
    #[serde(default)]
    pub attempts: u32,
    /// Absolute expiry as unix seconds (`scheduling.job_max_age_secs` at
    /// seed time). A job that sat in a shared queue past its freshness
    /// budget is dead-lettered on pop — a stale crawl is worse than a
    /// retried one. `None` (default / payloads from older seeds) = never.
    #[serde(default)]
    pub expires_at: Option<u64>,
}

/// Thin wrapper over a multiplexed Redis connection with harvest's key
/// layout. All methods are safe to interleave on one connection.
#[derive(Clone)]
pub struct Queue {
    con: redis::aio::MultiplexedConnection,
    prefix: String,
    /// Lease heartbeat TTL in seconds (tests shorten it; workers default).
    lease_ttl: u64,
}

impl Queue {
    pub async fn connect(redis_url: &str, prefix: &str) -> Result<Self> {
        // Keys are built by concatenation with this prefix: anything outside
        // the key-token charset could address another tenant's keyspace or
        // corrupt glob patterns. Refuse it before any I/O.
        if !is_safe_key_token(prefix) {
            anyhow::bail!(
                "queue prefix {prefix:?} must be a redis key token: 1..=128 bytes of                  letters, digits, '_', '-', ':'"
            );
        }
        let client = redis::Client::open(redis_url.to_string())
            .with_context(|| format!("bad redis url {redis_url}"))?;
        let con = client
            .get_multiplexed_async_connection()
            .await
            .context("redis connect failed")?;
        Ok(Self {
            con,
            prefix: prefix.to_string(),
            lease_ttl: DEFAULT_LEASE_TTL_SECS,
        })
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    fn k(&self, s: &str) -> String {
        format!("{}:{}", self.prefix, s)
    }

    pub async fn push_job(&mut self, job: &Job) -> Result<()> {
        let raw = serde_json::to_string(job)?;
        let _: () = self.con.lpush(self.k("jobs"), raw).await?;
        Ok(())
    }

    /// Pop the next job, atomically parking it in `inflight` until acked.
    /// Returns `None` after `timeout` of waiting (idle tick).
    pub async fn pop_job(&mut self, timeout: Duration) -> Result<Option<(String, Job)>> {
        let secs = timeout.as_secs().max(1);
        let raw: Option<String> = redis::cmd("BRPOPLPUSH")
            .arg(&[self.k("jobs"), self.k("inflight"), secs.to_string()])
            .query_async(&mut self.con)
            .await?;
        let Some(raw) = raw else { return Ok(None) };
        match serde_json::from_str::<Job>(&raw) {
            Ok(job) => Ok(Some((raw, job))),
            Err(e) => {
                // Corrupt payload: quarantine it, never block the queue.
                tracing::warn!(%e, "undecodable job; moving to dlq");
                let entry =
                    serde_json::json!({ "url": "", "reason": "corrupt-payload", "attempts": null });
                let _: () = self
                    .con
                    .rpush(self.k("dlq"), entry.to_string())
                    .await
                    .unwrap_or_default();
                let _: () = self
                    .con
                    .lrem(self.k("inflight"), 1, raw.clone())
                    .await
                    .unwrap_or_default();
                Ok(Some((
                    raw,
                    Job {
                        url: String::new(),
                        depth: 0,
                        target: 0,
                        expires_at: None,
                        attempts: u32::MAX,
                    },
                )))
            }
        }
    }

    pub async fn ack(&mut self, raw: &str) -> Result<()> {
        let _: i64 = self.con.lrem(self.k("inflight"), 1, raw).await?;
        Ok(())
    }

    /// Requeue with attempts+1 (fresh LPUSH; order among retries is FIFO-ish
    /// because the list is drained from the other end).
    pub async fn requeue(&mut self, mut job: Job) -> Result<()> {
        job.attempts += 1;
        self.push_job(&job).await
    }

    pub async fn dead(&mut self, job: &Job, reason: &str) -> Result<()> {
        let entry = serde_json::json!({
            "url": job.url,
            "target": self.prefix.clone(),
            "reason": reason,
            "attempts": job.attempts,
        });
        let _: () = self.con.rpush(self.k("dlq"), entry.to_string()).await?;
        Ok(())
    }

    /// Override the lease heartbeat TTL (short for tests, default for runs).
    pub fn with_lease_ttl(mut self, secs: u64) -> Self {
        self.lease_ttl = secs.max(1);
        self
    }

    pub fn lease_ttl(&self) -> Duration {
        Duration::from_secs(self.lease_ttl)
    }

    fn lease_key(&self, raw: &str) -> String {
        let d = sha2::Sha256::digest(raw.as_bytes());
        let mut s = String::with_capacity(24);
        for b in d.iter().take(12) {
            s.push_str(&format!("{b:02x}"));
        }
        format!("lease:{s}")
    }

    /// Claim/refresh the lease for an in-flight job. The value is the
    /// holder's LeaseId, so `redis-cli GET` shows exactly who is working on
    /// what. Lease heartbeats are advisory (the queue stays correct if the
    /// whole heartbeat path fails); errors here must never kill a worker.
    pub async fn touch_lease(&mut self, raw: &str, holder: &LeaseId) -> Result<()> {
        let _: () = redis::cmd("SETEX")
            .arg(&[
                self.k(&self.lease_key(raw)),
                self.lease_ttl.to_string(),
                holder.as_str().to_string(),
            ])
            .query_async(&mut self.con)
            .await?;
        Ok(())
    }

    pub async fn lease_holder(&mut self, raw: &str) -> Result<Option<String>> {
        Ok(self.con.get(self.k(&self.lease_key(raw))).await?)
    }

    pub async fn has_lease(&mut self, raw: &str) -> Result<bool> {
        Ok(self.lease_holder(raw).await?.is_some())
    }

    pub async fn drop_lease(&mut self, raw: &str) {
        let _: redis::RedisResult<i64> = self.con.del(self.k(&self.lease_key(raw))).await;
    }

    /// Add a member to a set; true means "first time we've seen this".
    pub async fn mark_seen(&mut self, set: &str, member: &str) -> Result<bool> {
        let added: i64 = self.con.sadd(self.k(set), member).await?;
        Ok(added == 1)
    }

    /// Shared per-host fixed-window limiter: at most `per_sec` requests per
    /// wall-clock second across *all* workers. Returns early if `stop` is
    /// set so shutdown stays prompt.
    pub async fn wait_host(
        &mut self,
        host: &str,
        per_sec: f64,
        stop: Option<&Arc<AtomicBool>>,
    ) -> Result<()> {
        if !per_sec.is_finite() || per_sec <= 0.0 {
            return Ok(());
        }
        let limit = per_sec.ceil().max(1.0) as i64;
        loop {
            let sec = now_secs();
            let key = format!("{}:rl:{}:{}", self.prefix, host, sec);
            let n: i64 = self.con.incr(&key, 1).await?;
            if n == 1 {
                let _: i64 = self.con.expire(&key, 2).await?;
            }
            if n <= limit {
                return Ok(());
            }
            if stop.map(|s| s.load(Ordering::SeqCst)).unwrap_or(false) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
    }

    /// Cached robots.txt for an origin. Ok(None) = "not cached yet";
    /// Ok(Some(None)) = "origin has no robots.txt" (allow-all).
    pub async fn robots_cached(&mut self, origin_key: &str) -> Result<Option<Option<String>>> {
        let v: Option<String> = self.con.get(self.k(origin_key)).await?;
        Ok(v.map(|s| if s.is_empty() { None } else { Some(s) }))
    }

    pub async fn robots_store(
        &mut self,
        origin_key: &str,
        txt: Option<&str>,
        ttl: u64,
    ) -> Result<()> {
        let v = txt.unwrap_or_default().to_string();
        let _: () = self.con.set_ex(self.k(origin_key), v, ttl).await?;
        Ok(())
    }

    pub async fn incr_budget(&mut self, key: &str) -> Result<i64> {
        let n: i64 = self.con.incr(self.k(key), 1).await?;
        Ok(n)
    }

    pub async fn counts(&mut self) -> Result<serde_json::Value> {
        let jobs: i64 = self.con.llen(self.k("jobs")).await?;
        let inflight: i64 = self.con.llen(self.k("inflight")).await?;
        let dlq: i64 = self.con.llen(self.k("dlq")).await?;
        let seen_urls: i64 = self.con.scard(self.k("seen_urls")).await?;
        let seen_fp: i64 = self.con.scard(self.k("seen_fp")).await?;
        Ok(serde_json::json!({
            "prefix": self.prefix,
            "jobs": jobs,
            "inflight": inflight,
            "dlq": dlq,
            "seen_urls": seen_urls,
            "seen_fp": seen_fp,
        }))
    }

    /// Move still-unacked inflight jobs back to `jobs`, EXCEPT ones whose
    /// lease heartbeat is fresh (a live worker owns those; touching them
    /// would double-process). Intended for the gap between stopping a cohort
    /// and starting the next one; with no workers running, leases have all
    /// expired, so everything is reclaimed. A job acked mid-sweep simply
    /// fails the LREM and is left alone.
    pub async fn sweep_inflight(&mut self) -> Result<Sweep> {
        let strays: Vec<String> = redis::cmd("LRANGE")
            .arg(&[self.k("inflight"), "0".to_string(), "-1".to_string()])
            .query_async(&mut self.con)
            .await?;
        let mut out = Sweep::default();
        for raw in strays {
            if self.has_lease(&raw).await.unwrap_or(false) {
                out.held_by_lease += 1;
                continue;
            }
            let removed: i64 = self.con.lrem(self.k("inflight"), 1, raw.clone()).await?;
            if removed == 0 {
                continue; // acked while we looked; not a stray
            }
            let _: () = self.con.lpush(self.k("jobs"), raw).await?;
            out.moved += 1;
        }
        Ok(out)
    }

    /// Delete every key under this prefix (dev/test convenience).
    pub async fn flush_prefix(&mut self) -> Result<()> {
        let keys: Vec<String> = redis::cmd("KEYS")
            .arg(&[format!("{}:*", self.prefix)])
            .query_async(&mut self.con)
            .await?;
        if !keys.is_empty() {
            let _: i64 = redis::cmd("DEL")
                .arg(&keys)
                .query_async(&mut self.con)
                .await?;
        }
        Ok(())
    }
}

/// What a stray-reclaim pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Sweep {
    pub moved: u64,
    pub held_by_lease: u64,
}

/// Default lease heartbeat TTL. A killed worker's leases expire after this
/// and its strays become reclaimable; a live worker refreshes at TTL/3.
pub const DEFAULT_LEASE_TTL_SECS: u64 = 30;

/// Per-worker run tally (printed on exit, returned to the caller).
#[derive(Debug, Default, Clone, Serialize)]
pub struct WorkerStats {
    pub fetched: u64,
    pub written: u64,
    pub deduped: u64,
    pub blocked: u64,
    pub failed: u64,
    pub robots_denied: u64,
    pub requeued: u64,
    pub dead: u64,
    /// Queue-protocol hiccups (pop/ack errors) that were tolerated with
    /// backoff instead of killing the worker. Nonzero means redis flaked.
    pub queue_errors: u64,
}

/// Seed the queue from config start URLs (and optionally the local dead-
/// letter file). Returns how many jobs were enqueued.
pub async fn push(cfg: &Config, redis_url: &str, prefix: &str) -> Result<usize> {
    let mut q = Queue::connect(redis_url, prefix).await?;
    let mut n = 0;
    for (i, t) in cfg.targets.iter().enumerate() {
        let u = Url::parse(&t.start_url).with_context(|| format!("target {}", t.id))?;
        if q.mark_seen("seen_urls", &dedup_key(&u)).await? {
            q.push_job(&Job {
                url: u.to_string(),
                depth: 0,
                target: i,
                attempts: 0,
                expires_at: expire_at(cfg),
            })
            .await?;
            n += 1;
        }
    }
    Ok(n)
}

pub struct WorkerOpts {
    pub redis_url: String,
    pub prefix: String,
    /// This process's stable name (lease holder ids + logs are keyed by it).
    /// Must be an id slug.
    pub name: String,
    /// Quit after this many consecutive idle pop timeouts.
    pub idle_secs: u64,
    /// Consecutive pop failures tolerated before the worker gives up
    /// (transient redis loss is backed off, not fatal — the multiplexed
    /// connection reconnects underneath).
    pub queue_fail_limit: u32,
    pub stop: Option<Arc<AtomicBool>>,
}

/// Drain the shared queue until idle/stop. `cfg.output` should be a
/// per-shard sink path (jsonl with a shard suffix is applied automatically
/// below) since each worker process writes independently.
pub async fn worker(cfg: Config, opts: WorkerOpts) -> Result<WorkerStats> {
    let mut cfg = cfg;
    cfg.validate()?;
    let worker_id = WorkerId::parse(&opts.name)
        .map_err(|e| anyhow::anyhow!("worker name {:?} is not usable: {e}", opts.name))?;
    // Queue owns retries; keep the HTTP fetcher strictly single-shot.
    let queue_budget = cfg.max_retries;
    cfg.max_retries = 0;
    // Shard-local sink file so concurrent appends never share a handle.
    shard_output(&mut cfg);
    let cfg = Arc::new(cfg);

    let metrics = Arc::new(Metrics::default());
    let pool = Arc::new(ProxyPool::new(&cfg.proxies));
    // Same per-egress budget as single-node crawls: a shard must not use the
    // queue as a license to hammer one proxy (0 = unlimited).
    pool.set_inflight_cap(cfg.max_inflight_per_proxy);
    let fetcher = Arc::new(Fetcher::new(cfg.clone(), pool.clone()));
    // Per-process engine runtime: the same breaker + event bus the single
    // node uses, scoped to this worker. There is no events *file* here
    // (many shards writing one path would interleave corrupt JSONL); the
    // ring powers debug output and keeps worker code on the shared path.
    let runtime = Runtime::new(&cfg, metrics.clone());
    let (sink, sink_path) = sink::build(&cfg.output)?;
    let extractors: Vec<Extractor> = cfg
        .targets
        .iter()
        .map(|t| Extractor::compile(&t.extract))
        .collect::<Result<_>>()?;
    // Same transform/schema pipeline as the single-node engine: a sharded
    // crawl must not write records its own `harvest validate` would flag.
    let pipelines: Vec<crate::transform::TargetPipeline> = cfg
        .targets
        .iter()
        .map(crate::transform::TargetPipeline::compile)
        .collect::<Result<_>>()?;

    let mut q = Queue::connect(&opts.redis_url, &opts.prefix).await?;
    let mut st = WorkerStats::default();
    let mut idle: u64 = 0;
    let mut pop_streak: u32 = 0;
    let mut lease_seq: u64 = 0;
    tracing::info!(worker = %worker_id, prefix = %opts.prefix, path = %sink_path.display(), "worker draining redis queue");

    loop {
        if opts
            .stop
            .as_ref()
            .map(|s| s.load(Ordering::SeqCst))
            .unwrap_or(false)
        {
            tracing::info!("stop requested; worker exiting");
            break;
        }
        let popped = match q.pop_job(Duration::from_secs(1)).await {
            Ok(p) => {
                pop_streak = 0;
                p
            }
            Err(e) => {
                // A flaky redis is backpressure, not a death sentence: the
                // multiplexed connection reconnects underneath, so back off
                // and keep draining until a hard limit says otherwise.
                st.queue_errors += 1;
                pop_streak += 1;
                tracing::warn!(%e, streak = pop_streak, "queue pop failed; backing off");
                if pop_streak >= opts.queue_fail_limit.max(1) {
                    anyhow::bail!(
                        "queue unavailable after {pop_streak} consecutive pop failures: {e:#}"
                    );
                }
                tokio::time::sleep(Duration::from_millis(
                    (250u64 << pop_streak.min(4)).min(4_000),
                ))
                .await;
                continue;
            }
        };
        let Some((raw, job)) = popped else {
            idle += 1;
            if idle >= opts.idle_secs.max(1) {
                break; // queue exhausted
            }
            continue;
        };
        idle = 0;
        // Lease the job while it runs (advisory, best-effort): sweep sees
        // `holder` and leaves the entry alone until this worker stops
        // heartbeating it.
        lease_seq += 1;
        let salt = (unix_now().wrapping_mul(0x9E37_79B9) ^ (lease_seq << 7)) as u32;
        let lease = LeaseId::new(&worker_id, lease_seq, salt);
        let _ = q.touch_lease(&raw, &lease).await;
        let hb_stop = Arc::new(AtomicBool::new(false));
        {
            let mut qhb = q.clone();
            let raw_hb = raw.clone();
            let lease_hb = lease.clone();
            let stop = hb_stop.clone();
            let every =
                Duration::from_secs(q.lease_ttl().as_secs() / 3).max(Duration::from_millis(100));
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(every).await;
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let _ = qhb.touch_lease(&raw_hb, &lease_hb).await;
                }
            });
        }
        if let Some(exp) = job.expires_at {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if now > exp {
                runtime.emit(
                    EventKind::JobExpired,
                    None,
                    serde_json::json!({"url": job.url, "target": job.target}),
                );
                q.dead(&job, "expired-in-queue").await?;
                st.dead += 1;
                continue;
            }
        }
        if let Err(e) = handle_job(
            &cfg,
            &fetcher,
            &metrics,
            &sink,
            &extractors,
            &pipelines,
            &mut q,
            &job,
            opts.stop.as_ref(),
            queue_budget,
            &runtime,
            &mut st,
        )
        .await
        {
            tracing::warn!(url = %job.url, %e, "job error -> dead-letter");
            if let Err(e) = q.dead(&job, &format!("worker:{e}")).await {
                st.queue_errors += 1;
                tracing::error!(%e, "dlq write failed (job lost from queue anyway)");
            }
            st.dead += 1;
        }
        hb_stop.store(true, Ordering::Relaxed);
        q.drop_lease(&raw).await;
        if let Err(e) = q.ack(&raw).await {
            st.queue_errors += 1;
            tracing::warn!(%e, "ack failed; inflight stray will be swept by lease expiry");
        }
    }
    sink.flush()?;
    tracing::info!(?st, "worker finished");
    Ok(st)
}

#[allow(clippy::too_many_arguments)]
async fn handle_job(
    cfg: &Arc<Config>,
    fetcher: &Arc<Fetcher>,
    metrics: &Arc<Metrics>,
    sink: &Arc<dyn sink::Sink>,
    extractors: &[Extractor],
    pipelines: &[crate::transform::TargetPipeline],
    q: &mut Queue,
    job: &Job,
    stop: Option<&Arc<AtomicBool>>,
    retry_budget: u32,
    runtime: &Arc<Runtime>,
    st: &mut WorkerStats,
) -> Result<()> {
    if job.url.is_empty() {
        st.dead += 1; // corrupt payload, already quarantined by pop_job
        return Ok(());
    }
    let url = match Url::parse(&job.url) {
        Ok(u) => u,
        Err(_) => return Ok(()),
    };
    let Some(tcfg) = cfg.targets.get(job.target) else {
        return Ok(());
    };
    let host = url.host_str().unwrap_or_default().to_string();

    if cfg.respect_robots && !robots_allows(cfg, fetcher, metrics, q, &url).await? {
        metrics.robot_denied();
        st.robots_denied += 1;
        return Ok(());
    }

    // Shared pacing for this host across all shards.
    let pol = cfg.policy_for(&host);
    q.wait_host(&host, pol.requests_per_sec, stop).await?;

    runtime.emit(
        EventKind::JobStarted,
        None,
        serde_json::json!({"url": job.url, "depth": job.depth, "target": job.target}),
    );

    // Transport breaker (per process — the queue is the shared state here;
    // a dead upstream should at least stop burning *this* shard's
    // connections. Requeue keeps the job alive for other workers/epochs.)
    let breaker = runtime.breaker_for(&host);
    if let Some(b) = &breaker {
        if let Permit::Open { retry_in } = b.permit(std::time::Instant::now()) {
            metrics.breaker_rejected();
            if job.expires_at.is_some() {
                // Age-bounded job: hand it back; the pop-time expiry check
                // guarantees the requeue loop terminates.
                q.requeue(job.clone()).await?;
                st.requeued += 1;
                tracing::debug!(host, ?retry_in, "transport breaker open; job requeued");
            } else {
                // No deadline => requeueing could hot-spin the queue while
                // the circuit stays open; retire it visibly instead.
                q.dead(job, "breaker:open").await?;
                st.dead += 1;
            }
            return Ok(());
        }
    }

    // Single-shot fetch under the engine's execute envelope: the plan
    // carries the queue-imposed deadline, the context carries events.
    // (Retry stays 0: the redis queue owns retry scheduling — see `worker`.)
    let mut plan = RequestPlan::new(url.clone());
    plan.max_response_bytes = cfg.max_response_bytes;
    plan.deadline = job.expires_at.and_then(|exp| {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let remaining = exp.saturating_sub(now);
        Some(std::time::Instant::now() + Duration::from_secs(remaining))
    });
    let rctx = RequestContext {
        correlation_id: st.fetched,
        job_id: None,
        target: job.target,
        host: host.clone(),
        bus: Some(runtime.bus.clone()),
        acct: None,
        budget: None,
    };
    let outcome = match fetcher.execute(plan, &rctx, metrics).await {
        Ok((o, _meta)) => {
            if let Some(b) = &breaker {
                b.note_success(std::time::Instant::now());
            }
            o
        }
        Err((e, _meta)) => {
            if let Some(b) = &breaker {
                if matches!(e, FetchError::Failed(_)) {
                    b.note_failure(std::time::Instant::now());
                } else {
                    b.note_success(std::time::Instant::now());
                }
            }
            match e {
                FetchError::Blocked { status, .. } if matches!(status, 401 | 403 | 451) => {
                    metrics.request_blocked();
                    q.dead(job, &format!("refused:{status}")).await?;
                    st.blocked += 1;
                    st.dead += 1;
                    return Ok(());
                }
                FetchError::Blocked {
                    status,
                    retry_after,
                } => {
                    metrics.request_blocked();
                    if let Some(d) = retry_after {
                        tokio::time::sleep(d.min(Duration::from_secs(30))).await;
                    }
                    if job.attempts < retry_budget {
                        q.requeue(job.clone()).await?;
                        st.requeued += 1;
                    } else {
                        q.dead(job, &format!("blocked:{status}")).await?;
                        st.blocked += 1;
                        st.dead += 1;
                    }
                    return Ok(());
                }
                FetchError::Failed(msg) => {
                    if job.attempts < retry_budget {
                        q.requeue(job.clone()).await?;
                        st.requeued += 1;
                    } else {
                        q.dead(job, &format!("failed:{msg}")).await?;
                        st.failed += 1;
                        st.dead += 1;
                    }
                    return Ok(());
                }
                FetchError::Permanent(msg) => {
                    // A local safety limit (body cap, bad proxy config) will not
                    // change on retry: dead-letter with the reason, do not requeue.
                    st.failed += 1;
                    q.dead(job, &format!("permanent:{msg}")).await?;
                    return Ok(());
                }
                FetchError::RobotsDenied => return Ok(()),
            }
        }
    };
    metrics.page_fetched();

    let is_text =
        outcome.content_type.contains("html") || outcome.content_type.starts_with("text/");
    if !is_text {
        return Ok(());
    }
    let fp = extract::fingerprint(&outcome.body);
    if !q
        .mark_seen("seen_fp", &format!("{}:{}", tcfg.id, fp))
        .await?
    {
        metrics.record_deduped();
        st.deduped += 1;
        return Ok(());
    }

    let (fields, hrefs, quarantined) = {
        let doc = Html::parse_document(&outcome.body);
        let mut fields = extractors[job.target].extract(&doc);
        let po = pipelines[job.target].apply(&mut fields, url.as_str(), job.depth)?;
        for _ in 0..po.drift {
            metrics.schema_drift();
        }
        if po.failures > 0 {
            metrics.pipeline_failures(po.failures);
        }
        if let Some(reasons) = &po.quarantined {
            metrics.record_quarantined();
            tracing::warn!(
                url = %url,
                target = %tcfg.id,
                "record quarantined by schema enforcement: {}",
                reasons.join("; ")
            );
        }
        let hrefs = if tcfg.follow_links && job.depth < tcfg.max_depth {
            let sel: Selector = tcfg
                .link_selector
                .as_deref()
                .and_then(|s| Selector::parse(s).ok())
                .unwrap_or_else(|| Selector::parse("a[href]").expect("validated at startup"));
            extract::links(&doc, &sel)
        } else {
            Vec::new()
        };
        (fields, hrefs, po.quarantined)
    };
    let rec = Record {
        run: cfg.name.clone(),
        target: tcfg.id.clone(),
        url: url.to_string(),
        depth: job.depth,
        ts: now_secs(),
        fingerprint: fp,
        egress: outcome.proxy_id.clone(),
        fields,
    };
    metrics.record_extracted();
    runtime.emit(
        EventKind::ExtractionCompleted,
        None,
        serde_json::json!({"url": rec.url, "fields": rec.fields.len()}),
    );
    if quarantined.is_none() {
        let t_write = std::time::Instant::now();
        let wr = sink.write(&rec);
        metrics.note_storage_write(t_write.elapsed().as_millis() as u64);
        wr?;
        runtime.emit(
            EventKind::SinkWriteCompleted,
            None,
            serde_json::json!({"url": rec.url, "fingerprint": rec.fingerprint}),
        );
        metrics.record_written();
        if let Some(t) = cfg.targets.get(job.target) {
            metrics.note_target_record(&t.id);
        }
        st.written += 1;
    }
    st.fetched += 1;

    // Budgets: global max_pages and per-target max_pages are enforced
    // through shared Redis counters (approximate under races — fine for a
    // politeness budget; the per-host limiter is what sites actually feel).
    let g = q.incr_budget("budget").await?;
    let t = q.incr_budget(&format!("tgt:{}", tcfg.id)).await?;
    if g >= cfg.max_pages as i64 || t >= tcfg.max_pages as i64 {
        return Ok(());
    }

    if tcfg.follow_links && job.depth < tcfg.max_depth {
        for href in hrefs {
            let Ok(abs) = url.join(href.trim()) else {
                continue;
            };
            if abs.scheme() != url.scheme()
                || abs.host_str() != url.host_str()
                || abs.port() != url.port()
            {
                continue;
            }
            if !tcfg.include.is_empty()
                && !tcfg
                    .include
                    .iter()
                    .any(|p| abs.as_str().contains(p.as_str()))
            {
                continue;
            }
            if tcfg
                .exclude
                .iter()
                .any(|p| abs.as_str().contains(p.as_str()))
            {
                continue;
            }
            if !q.mark_seen("seen_urls", &dedup_key(&abs)).await? {
                continue;
            }
            if cfg.respect_robots && !robots_allows(cfg, fetcher, metrics, q, &abs).await? {
                continue;
            }
            q.push_job(&Job {
                url: abs.to_string(),
                depth: job.depth + 1,
                target: job.target,
                attempts: 0,
                expires_at: expire_at(cfg),
            })
            .await?;
        }
    }
    Ok(())
}

/// robots.txt check with the fetch result cached in Redis under the origin
/// key, so the fleet asks each site once per TTL, not once per worker.
async fn robots_allows(
    cfg: &Arc<Config>,
    fetcher: &Arc<Fetcher>,
    metrics: &Arc<Metrics>,
    q: &mut Queue,
    url: &Url,
) -> Result<bool> {
    let origin_key = format!("robots:{}://{}", url.scheme(), url.authority());
    let txt = match q.robots_cached(&origin_key).await? {
        Some(cached) => {
            metrics.robot_cache_hit();
            cached
        }
        None => {
            let base = Url::parse(&format!("{}://{}", url.scheme(), url.authority()))
                .unwrap_or_else(|_| url.clone());
            let fresh = fetcher.fetch_robots(&base, metrics).await;
            q.robots_store(&origin_key, fresh.as_deref(), cfg.robots_ttl_secs)
                .await?;
            fresh
        }
    };
    let robots = match &txt {
        Some(t) => Robots::parse(t, &ua_token(&cfg.user_agent)),
        None => Robots::default(),
    };
    Ok(robots.allows(url.path()))
}

/// Give each worker process its own jsonl/csv fragment (jsonl shards merge
/// with `cat`; sqlite/postgres handles are already safe to share).
fn shard_output(cfg: &mut Config) {
    let old = match &cfg.output {
        crate::config::SinkCfg::Jsonl { path } | crate::config::SinkCfg::Csv { path } => {
            Some(path.clone())
        }
        _ => None,
    };
    if let Some(p) = old {
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("log")
            .to_string();
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("out")
            .to_string();
        let new = p.with_file_name(format!("{stem}.p{}.{}", std::process::id(), ext));
        cfg.output = match cfg.output {
            crate::config::SinkCfg::Jsonl { .. } => crate::config::SinkCfg::Jsonl { path: new },
            _ => crate::config::SinkCfg::Csv { path: new },
        };
    }
}

fn dedup_key(u: &Url) -> String {
    let host = u.host_str().unwrap_or("");
    let q = u.query().map(|s| format!("?{s}")).unwrap_or_default();
    format!("{host}{}{q}", u.path())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
