//! Execution coordinator: accepted commands in, bounded jobs out.
//!
//! This module is the seam between the typed command model
//! ([`crate::control`]) and the machinery that already knows how to do the
//! work. It owns **no** crawl logic, **no** export logic, **no** AI logic and
//! **no** queue: it hands a crawl to [`crate::engine`], an export to
//! [`crate::export_job`] (which is a thin wrapper over [`crate::export`]), an
//! AI round to [`crate::ai_job`] (over [`crate::ai`]), and uses the process's
//! tokio runtime for concurrency.
//!
//! What it does own is the contract around execution:
//!
//! * **Capacity.** At most [`crate::control::MAX_CONCURRENT_JOBS`] jobs run
//!   at once and at most [`crate::control::MAX_QUEUE_DEPTH`] are in flight
//!   (queued + running). Exceeding either is a clean 503, never a silent
//!   queue that grows without bound.
//! * **A deadline.** Every job carries `created_ts + limits.timeout_secs`.
//!   The running task races that deadline, and `get`/`list` re-check it, so a
//!   task that dies without reporting still cannot leave a job `running`
//!   forever.
//! * **Cancellation.** Each job owns an `AtomicBool` that is handed to the
//!   engine (which drains politely) and checked between steps by the
//!   blocking jobs. Cancelling a queued job means it never runs.
//! * **Audit.** Submit, start, finish, cancel and every refusal that created
//!   a command write one safe record: operation, actor, resource id,
//!   correlation id, result, timestamp. No tokens, no paths, no payloads.
//! * **Deterministic transitions.** [`ExecState::can_transition`] is the only
//!   way a job changes state; illegal moves are refused, not absorbed.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::api::epoch_secs;
use crate::config::Config;
use crate::control::{
    CommandLimits, CommandSpec, ControlCommand, ControlError, ControlJobId, IdempotencyIndex,
    Operation, MAX_CONCURRENT_JOBS, MAX_QUEUE_DEPTH,
};
use crate::download::{ArtifactRole, ArtifactStore, VerificationState};
use crate::ops::{AuditLedger, OpsLog};
use crate::runtime::RuntimeSlot;

/// Retained job records (terminal ones are evicted oldest-first).
pub const MAX_JOBS: usize = 256;

/// Lifecycle of one control job.
///
/// Separate from [`crate::job::JobState`] on purpose: that state machine
/// describes a single URL fetch inside a run, this one describes a whole
/// operator-initiated round. The two never mix, and this one allows
/// `Queued -> Failed` because a job that cannot even start (bad target,
/// export source missing) must be able to say so instead of pretending it
/// never happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecState {
    /// Accepted, waiting for a execution slot.
    #[default]
    Queued,
    /// Running under the engine/exporter/provider.
    Running,
    /// Finished successfully.
    Completed,
    /// Finished with a recorded reason.
    Failed,
    /// Cancelled before, or at the next checkpoint of, its work.
    Cancelled,
}

impl ExecState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
    pub fn in_flight(self) -> bool {
        matches!(self, Self::Queued | Self::Running)
    }
    /// The only legal moves.
    pub fn can_transition(from: ExecState, to: ExecState) -> bool {
        use ExecState as S;
        matches!(
            (from, to),
            (S::Queued, S::Running)
                | (S::Queued, S::Failed)
                | (S::Queued, S::Cancelled)
                | (S::Running, S::Completed)
                | (S::Running, S::Failed)
                | (S::Running, S::Cancelled)
        )
    }
}

/// Counters a job reports. Every field is either read back from the engine's
/// own summary or left at zero; nothing is estimated.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCounters {
    pub pages_fetched: u64,
    pub requests_total: u64,
    pub records_extracted: u64,
    pub records_written: u64,
    pub records_deduped: u64,
    pub retries: u64,
    pub failures: u64,
    pub quarantined: u64,
    pub schema_drifts: u64,
    pub blocked: u64,
}

impl JobCounters {
    pub fn from_summary(s: &crate::engine::Summary) -> Self {
        Self {
            pages_fetched: s.pages_fetched,
            requests_total: s.requests_total,
            records_extracted: s.records,
            records_written: s.written,
            records_deduped: s.deduped,
            retries: s.retries,
            failures: s.per_target.values().map(|t| t.failed).sum(),
            quarantined: s.quarantined,
            schema_drifts: s.schema_drifts,
            blocked: s.blocked,
        }
    }
}

/// One line in a job's event trail. Bounded by [`MAX_EVENTS`] per job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobEvent {
    pub ts: u64,
    pub kind: String,
    pub detail: Value,
}

/// Retained events per job.
pub const MAX_EVENTS: usize = 32;

#[derive(Debug, Clone, Serialize)]
pub struct JobSnapshot {
    pub id: String,
    pub command: String,
    pub corr: String,
    pub operation: String,
    pub state: String,
    pub created_ts: u64,
    pub updated_ts: u64,
    /// Absolute deadline (epoch seconds). The client does not have to guess
    /// how long a job may still take.
    pub deadline_ts: u64,
    /// Safe resource label: target ids, format list, ai task — never a path.
    pub resource: Option<String>,
    pub limits: CommandLimits,
    pub counters: JobCounters,
    pub artifacts: Vec<Value>,
    /// Operation-specific result (export summary, ai outcome, crawl summary).
    pub result: Value,
    pub error: Option<String>,
    pub cancel_requested: bool,
    pub events: Vec<JobEvent>,
    /// How to find this action in the audit ledger: the correlation id is the
    /// join key, and it is the only thing we hand out.
    pub audit_ref: String,
}

/// Bounded list filter.
#[derive(Debug, Clone, Default)]
pub struct JobFilter {
    pub state: Option<ExecState>,
    pub op: Option<Operation>,
    pub limit: usize,
    pub offset: usize,
}

impl JobFilter {
    /// Clamp to safe pagination bounds, matching the rest of the admin API.
    pub fn clamped(&self) -> (usize, usize) {
        let limit = self.limit.clamp(1, 200);
        (limit, self.offset)
    }
}

/// A failed job can still carry the result it managed to produce (an AI
/// round that ran but was refused, for example). Reporting it is the point:
/// "the provider said disabled" is more useful than "failed".
#[derive(Debug, Clone, Default)]
pub struct JobFailure {
    pub reason: String,
    pub partial: Value,
}

impl JobFailure {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            partial: Value::Null,
        }
    }
    pub fn with_partial(reason: impl Into<String>, partial: Value) -> Self {
        Self {
            reason: reason.into(),
            partial,
        }
    }
}

struct Job {
    id: String,
    cmd: ControlCommand,
    state: ExecState,
    created_ts: u64,
    updated_ts: u64,
    deadline_ts: u64,
    cancel: Arc<AtomicBool>,
    counters: JobCounters,
    artifacts: Vec<String>,
    result: Value,
    error: Option<String>,
    events: Vec<JobEvent>,
    seq: u64,
}

/// Outcome of a submission, including whether it was a replay.
#[derive(Debug, Clone)]
pub struct SubmitOutcome {
    pub job: JobSnapshot,
    /// True when an `Idempotency-Key` matched an earlier identical request:
    /// the returned job is the original one and no second job was created.
    pub replayed: bool,
}

/// The coordinator. Cheap to clone into spawned tasks (`Arc`).
pub struct ExecutionRegistry {
    inner: Mutex<RegistryInner>,
    artifacts: Arc<ArtifactStore>,
    idempotency: IdempotencyIndex,
    audit: Arc<AuditLedger>,
    ops: Arc<OpsLog>,
    runtime: RuntimeSlot,
    root: PathBuf,
    seq: std::sync::atomic::AtomicU64,
}

#[derive(Default)]
struct RegistryInner {
    jobs: BTreeMap<String, Job>,
    cfg: Option<Arc<Config>>,
}

impl ExecutionRegistry {
    pub fn new(
        root: PathBuf,
        artifacts: Arc<ArtifactStore>,
        audit: Arc<AuditLedger>,
        ops: Arc<OpsLog>,
        runtime: RuntimeSlot,
    ) -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
            artifacts,
            idempotency: IdempotencyIndex::new(),
            audit,
            ops,
            runtime,
            root,
            seq: std::sync::atomic::AtomicU64::new(1),
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    /// Attach the effective configuration. Crawl scoping, export source and
    /// AI run-output jobs all read it; without one they report 503 instead of
    /// guessing.
    pub fn set_config(&self, cfg: Arc<Config>) {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).cfg = Some(cfg);
    }

    fn cfg(&self) -> Option<Arc<Config>> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cfg
            .clone()
    }

    // --------------------------------------------------------------- submit

    /// Validate, deduplicate and queue a command, then start it if a slot is
    /// free. Returns the job as it was created (state `queued` or `running`).
    pub fn submit(self: &Arc<Self>, cmd: &ControlCommand) -> Result<SubmitOutcome, ControlError> {
        let cfg = self.cfg();
        cmd.validate(cfg.as_deref())?;

        if let Some(key) = &cmd.idempotency_key {
            match self
                .idempotency
                .check(&cmd.actor, cmd.operation(), key, &cmd.body_hash)
            {
                crate::control::Idempotency::Replay(job_id) => {
                    let snap = self.get(&job_id).ok_or_else(|| {
                        ControlError::conflict(
                            "idempotency key refers to a job that is no longer retained",
                        )
                    })?;
                    self.audit.note(
                        &cmd.actor,
                        &format!("control.{}.replay", cmd.operation().as_str()),
                        "replayed",
                        cmd.audit_target().as_deref(),
                        &cmd.corr,
                    );
                    return Ok(SubmitOutcome {
                        job: snap,
                        replayed: true,
                    });
                }
                crate::control::Idempotency::Conflict => {
                    return Err(ControlError::conflict(
                        "this Idempotency-Key was already used with a different body",
                    ))
                }
                crate::control::Idempotency::Fresh => {}
            }
        }

        let id = self.enqueue(cmd)?;
        if let Some(key) = &cmd.idempotency_key {
            self.idempotency.bind(
                &cmd.actor,
                cmd.operation(),
                key,
                &cmd.body_hash,
                id.as_str(),
            );
        }
        // The audit line carries the decisions an auditor would want to see
        // later: for an AI job that is which provider was named and whether
        // external processing was consented to. Never the fields themselves.
        let detail = match &cmd.spec {
            crate::control::CommandSpec::AiAssist(a) => Some(json!({
                "provider": a.provider,
                "external_allowed": a.external_allowed,
                "source": match &a.source {
                    crate::control::AiSource::RunOutput { .. } => "run_output",
                    crate::control::AiSource::Inline { .. } => "inline",
                },
            })),
            _ => None,
        };
        self.audit.note_with_detail(
            &cmd.actor,
            &format!("control.{}.submit", cmd.operation().as_str()),
            "accepted",
            cmd.audit_target().as_deref(),
            &cmd.corr,
            detail,
        );
        self.pump();
        let job = self
            .get(id.as_str())
            .ok_or_else(|| ControlError::unavailable("job vanished during submission"))?;
        Ok(SubmitOutcome {
            job,
            replayed: false,
        })
    }

    /// Create the job record in `Queued`. Capacity is checked here: an
    /// exhausted control plane refuses rather than queueing forever.
    pub fn enqueue(&self, cmd: &ControlCommand) -> Result<ControlJobId, ControlError> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let in_flight = g.jobs.values().filter(|j| j.state.in_flight()).count();
        if in_flight >= MAX_QUEUE_DEPTH {
            return Err(ControlError::unavailable(format!(
                "the control plane is at capacity ({MAX_QUEUE_DEPTH} jobs in flight)"
            )));
        }
        let id = ControlJobId::next();
        let now = epoch_secs();
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let deadline = now.saturating_add(cmd.limits.timeout_secs);
        let job = Job {
            id: id.as_str().to_string(),
            cmd: cmd.clone(),
            state: ExecState::Queued,
            created_ts: now,
            updated_ts: now,
            deadline_ts: deadline,
            cancel: Arc::new(AtomicBool::new(false)),
            counters: JobCounters::default(),
            artifacts: Vec::new(),
            result: Value::Null,
            error: None,
            events: vec![JobEvent {
                ts: now,
                kind: "queued".to_string(),
                detail: json!({ "op": cmd.operation().as_str(), "deadline_ts": deadline }),
            }],
            seq,
        };
        g.jobs.insert(id.as_str().to_string(), job);
        Self::evict_locked(&mut g);
        Ok(id)
    }

    /// Start queued jobs while a slot is free. Called after every submission
    /// and every terminal transition, so nothing needs a background poller.
    fn pump(self: &Arc<Self>) {
        let mut started_any = true;
        while started_any {
            started_any = false;
            let next = {
                let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                let running = g
                    .jobs
                    .values()
                    .filter(|j| j.state == ExecState::Running)
                    .count();
                if running >= MAX_CONCURRENT_JOBS {
                    None
                } else {
                    g.jobs
                        .values()
                        .filter(|j| j.state == ExecState::Queued)
                        .min_by_key(|j| j.seq)
                        .map(|j| j.id.clone())
                }
            };
            if let Some(id) = next {
                self.start(&id);
                started_any = true;
            }
        }
    }

    /// Move one job `Queued -> Running` and spawn it.
    fn start(self: &Arc<Self>, id: &str) {
        let cancel = {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(job) = g.jobs.get_mut(id) else {
                return;
            };
            if !ExecState::can_transition(job.state, ExecState::Running) {
                return;
            }
            job.state = ExecState::Running;
            job.updated_ts = epoch_secs();
            push_event(job, "started", json!({}));
            job.cancel.clone()
        };
        let op = {
            let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            g.jobs.get(id).map(|j| j.cmd.operation())
        };
        let Some(op) = op else { return };
        self.audit.note(
            "admin-api",
            &format!("control.{}.start", op.as_str()),
            "running",
            Some(id),
            &self.corr_of(id),
        );
        let me = self.clone();
        let job_id = id.to_string();
        let handle = tokio::spawn(async move {
            me.run_job(job_id.clone(), cancel).await;
            me.pump();
        });
        // Detached on purpose: the job owns its own terminal transition, and
        // the deadline reaper covers a task that dies without reporting.
        drop(handle);
    }

    fn corr_of(&self, id: &str) -> String {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .jobs
            .get(id)
            .map(|j| j.cmd.corr.clone())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------ execution

    async fn run_job(self: &Arc<Self>, id: String, cancel: Arc<AtomicBool>) {
        let (spec, limits, timeout_secs) = {
            let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            match g.jobs.get(&id) {
                Some(j) => (j.cmd.spec.clone(), j.cmd.limits, j.cmd.limits.timeout_secs),
                None => return,
            }
        };

        let work = self.execute(&id, spec, limits, cancel.clone());
        let outcome =
            match tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), work).await {
                Ok(o) => o,
                Err(_) => {
                    // Deadline hit: set the stop flag so the engine drains, and
                    // record the timeout as the reason.
                    cancel.store(true, Ordering::Relaxed);
                    Err(JobFailure::new(format!(
                        "job exceeded its {}s deadline and was stopped",
                        timeout_secs
                    )))
                }
            };

        self.finish(&id, outcome, cancel.clone());
    }

    /// Run the actual work for a job. Returns `Err(reason)` for a failure and
    /// `Ok(result_json)` for a success; cancellation is detected through the
    /// shared flag.
    async fn execute(
        self: &Arc<Self>,
        id: &str,
        spec: CommandSpec,
        limits: CommandLimits,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, JobFailure> {
        if cancel.load(Ordering::Relaxed) {
            return Err(JobFailure::new("cancelled before it started"));
        }
        match spec {
            CommandSpec::Crawl(req) => self.run_crawl(id, &req, &limits, cancel).await,
            CommandSpec::Export(req) => self.run_export(id, &req, &limits, cancel).await,
            CommandSpec::AiAssist(req) => self.run_ai(id, &req, &limits).await,
            // Verify and artifact-inspect are answered synchronously by the
            // router; they never become jobs.
            CommandSpec::Verify(_) | CommandSpec::ArtifactInspect(_) => {
                Err(JobFailure::new("this operation does not create a job"))
            }
        }
    }

    async fn run_crawl(
        self: &Arc<Self>,
        id: &str,
        req: &crate::control::CrawlRequest,
        limits: &CommandLimits,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, JobFailure> {
        let base = self
            .cfg()
            .ok_or_else(|| JobFailure::new("no configuration is attached to this server"))?;
        let mut cfg = (*base).clone();
        if !req.targets.is_empty() {
            cfg.targets.retain(|t| req.targets.contains(&t.id));
        }
        if cfg.targets.is_empty() {
            return Err(JobFailure::new("no configured target matched this command"));
        }
        // Tighten only: a control command may never widen the audited
        // configuration's budgets.
        cfg.max_pages = match cfg.max_pages {
            0 => limits.max_pages,
            n => n.min(limits.max_pages),
        };
        cfg.max_records_per_run = match cfg.max_records_per_run {
            0 => limits.max_records,
            n => n.min(limits.max_records),
        };
        for t in &mut cfg.targets {
            t.max_depth = t.max_depth.min(limits.max_depth as usize);
        }

        let cfg = Arc::new(cfg);
        let metrics = Arc::new(crate::metrics::Metrics::default());
        let pool = Arc::new(crate::proxy::ProxyPool::new(&cfg.proxies));
        // Its own runtime slot: a control-plane crawl must not overwrite the
        // slot the serve loop publishes through `/runtime`.
        let runtime = RuntimeSlot::default();
        let ctx = crate::engine::RunCtx {
            metrics,
            pool,
            runtime,
        };
        let summary = crate::engine::run_with(cfg.clone(), ctx, Some(cancel.clone())).await;
        if cancel.load(Ordering::Relaxed) {
            return Err(JobFailure::new("cancelled during the run"));
        }
        let summary = summary.map_err(|e| JobFailure::new(format!("crawl failed: {e:#}")))?;

        // The run is a run: it lands in the same history ledger the scheduler
        // writes, with the same entry shape and policy hash.
        let entry = crate::runner::history_entry(&cfg, &summary, "ok");
        crate::runner::record_history(&cfg, &entry)
            .map_err(|e| JobFailure::new(format!("could not record run history: {e:#}")))?;

        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(job) = g.jobs.get_mut(id) {
                job.counters = JobCounters::from_summary(&summary);
            }
        }

        // The run output is an artifact when it is a file under the control
        // root — that is what "download the records" means here.
        if let Ok(files) = crate::export_job::source_files(&cfg) {
            for f in files {
                let name = f
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "records.jsonl".to_string());
                if let Ok(art) = self.artifacts.register(
                    f.clone(),
                    name,
                    ArtifactRole::RunOutput,
                    Some(id.to_string()),
                    VerificationState::NotRequested,
                ) {
                    let sha = std::fs::read(&f)
                        .map(|b| crate::control::sha256_hex(&b))
                        .unwrap_or_default();
                    let _ = self.artifacts.finalize(art.as_str(), sha);
                    let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                    if let Some(job) = g.jobs.get_mut(id) {
                        job.artifacts.push(art.as_str().to_string());
                    }
                }
            }
        }

        Ok(serde_json::to_value(&summary).unwrap_or(Value::Null))
    }

    async fn run_export(
        self: &Arc<Self>,
        id: &str,
        req: &crate::control::ExportRequest,
        limits: &CommandLimits,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, JobFailure> {
        let spec = crate::export_job::ExportJobSpec::from_request(req, limits)
            .map_err(|e| JobFailure::new(e.message))?;
        if cancel.load(Ordering::Relaxed) {
            return Err(JobFailure::new("cancelled before it started"));
        }
        let cfg = self
            .cfg()
            .ok_or_else(|| JobFailure::new("no configuration is attached to this server"))?;
        let out_dir = self.root.join("deliveries").join(id);
        let spec2 = spec.clone();
        let out = out_dir.clone();
        let cfg2 = cfg.clone();
        // `export::run` is synchronous file I/O; it goes to the blocking pool
        // so a big delivery cannot stall the admin server's async tasks.
        let res = tokio::task::spawn_blocking(move || crate::export_job::run(&spec2, &cfg2, &out))
            .await
            .map_err(|e| JobFailure::new(format!("export task failed: {e}")))?;

        let (result, produced) = res.map_err(JobFailure::new)?;
        if cancel.load(Ordering::Relaxed) {
            // Cancellation at the last checkpoint: nothing is registered, so
            // a cancelled export never becomes a downloadable artifact.
            self.artifacts.fail_job_artifacts(id, "job was cancelled");
            return Err(JobFailure::new("cancelled while finishing the delivery"));
        }

        let mut artifact_views: Vec<Value> = Vec::new();
        for f in produced {
            match self.artifacts.register(
                f.path.clone(),
                f.name.clone(),
                f.role,
                Some(id.to_string()),
                result.verification.clone(),
            ) {
                Ok(art) => {
                    let _ = self.artifacts.finalize(art.as_str(), f.sha256.clone());
                    if let Some(rec) = self.artifacts.get(art.as_str()) {
                        artifact_views.push(rec.to_json());
                        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(job) = g.jobs.get_mut(id) {
                            job.artifacts.push(art.as_str().to_string());
                        }
                    }
                }
                Err(e) => {
                    // A file that will not register is a failed job, not a
                    // silently missing artifact.
                    return Err(JobFailure::new(e.message));
                }
            }
        }

        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(job) = g.jobs.get_mut(id) {
                job.counters.records_written = result.records;
                job.counters.records_extracted = result.records;
            }
        }

        Ok(json!({
            "records": result.records,
            "unique_urls": result.unique_urls,
            "formats": result.formats,
            "files": result.files,
            "total_bytes": result.total_bytes,
            "verification": result.verification.to_json(),
            "delivery": result.delivery,
            "artifacts": artifact_views,
        }))
    }

    async fn run_ai(
        self: &Arc<Self>,
        id: &str,
        req: &crate::control::AiRequest,
        limits: &CommandLimits,
    ) -> Result<Value, JobFailure> {
        let spec = crate::ai_job::AiJobSpec::from_request(req, limits)
            .map_err(|e| JobFailure::new(e.message))?;
        let cfg = self
            .cfg()
            .ok_or_else(|| JobFailure::new("no configuration is attached to this server"))?;
        let cfg2 = cfg.clone();
        let spec2 = spec.clone();
        let res = tokio::task::spawn_blocking(move || crate::ai_job::run(&spec2, &cfg2))
            .await
            .map_err(|e| JobFailure::new(format!("ai task failed: {e}")))?;
        {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(job) = g.jobs.get_mut(id) {
                job.counters.records_extracted = res.counts.fields as u64;
            }
        }
        let value = serde_json::to_value(&res).unwrap_or(Value::Null);
        // A refused or failed AI round is a failed job: the console must be
        // able to say "the provider said no" instead of showing an empty
        // success card.
        if !res.ok() {
            return Err(JobFailure::with_partial(
                res.reason.clone().unwrap_or_else(|| res.state.clone()),
                value,
            ));
        }
        Ok(value)
    }

    // ------------------------------------------------------------ lifecycle

    fn finish(
        self: &Arc<Self>,
        id: &str,
        outcome: Result<Value, JobFailure>,
        cancel: Arc<AtomicBool>,
    ) {
        let cancelled = cancel.load(Ordering::Relaxed);
        let mut target = ExecState::Completed;
        let mut error: Option<String> = None;
        let mut result = Value::Null;
        match outcome {
            Ok(v) => result = v,
            Err(e) => {
                target = ExecState::Failed;
                error = Some(e.reason);
                if !e.partial.is_null() {
                    result = e.partial;
                }
            }
        }
        if cancelled {
            target = ExecState::Cancelled;
            error = Some(
                error.unwrap_or_else(|| "cancelled; partial work may have completed".to_string()),
            );
        }
        let corr = self.corr_of(id);
        let op = {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(job) = g.jobs.get_mut(id) else {
                return;
            };
            if job.state.is_terminal() {
                return;
            }
            if !ExecState::can_transition(job.state, target) {
                // Should be unreachable (Running/Cancelled is legal) but a
                // job must never be left non-terminal by a bad transition.
                target = ExecState::Failed;
                error = Some(format!("illegal transition to {}", target.as_str()));
            }
            job.state = target;
            job.updated_ts = epoch_secs();
            job.error = error.clone();
            job.result = result.clone();
            push_event(
                job,
                target.as_str(),
                json!({ "error": error.clone().unwrap_or_default() }),
            );
            job.cmd.operation()
        };
        if cancelled || target == ExecState::Failed {
            self.artifacts
                .fail_job_artifacts(id, "the producing job did not complete");
        }
        self.audit.note(
            "admin-api",
            &format!("control.{}.finish", op.as_str()),
            target.as_str(),
            Some(id),
            &corr,
        );
        self.ops.record(
            &self.runtime,
            &format!("control.{}", op.as_str()),
            json!({ "job": id, "state": target.as_str(), "corr": corr }),
        );
    }

    /// Cancel a job. Legal while queued (it never starts) and while running
    /// (the flag stops new work at the next checkpoint). Terminal jobs are a
    /// 409: there is nothing left to stop.
    pub fn cancel(&self, id: &str, actor: &str) -> Result<JobSnapshot, ControlError> {
        let Some(_parsed) = ControlJobId::parse(id) else {
            return Err(ControlError::not_found("no such job"));
        };
        let corr = {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            let Some(job) = g.jobs.get_mut(id) else {
                return Err(ControlError::not_found("no such job"));
            };
            if job.state.is_terminal() {
                return Err(ControlError::conflict(format!(
                    "job {id} is already {}; there is nothing left to cancel",
                    job.state.as_str()
                )));
            }
            job.cancel.store(true, Ordering::Relaxed);
            job.updated_ts = epoch_secs();
            push_event(job, "cancel_requested", json!({ "actor": actor }));
            job.cmd.corr.clone()
        };
        // A queued job can be finished immediately: it will never run.
        let is_queued = {
            let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            g.jobs
                .get(id)
                .map(|j| j.state == ExecState::Queued)
                .unwrap_or(false)
        };
        if is_queued {
            let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(job) = g.jobs.get_mut(id) {
                job.state = ExecState::Cancelled;
                job.updated_ts = epoch_secs();
                job.error = Some("cancelled before it started".to_string());
                push_event(job, "cancelled", json!({ "actor": actor }));
            }
        }
        self.audit.note(
            actor,
            "control.cancel",
            if is_queued {
                "cancelled"
            } else {
                "cancel_requested"
            },
            Some(id),
            &corr,
        );
        self.get(id)
            .ok_or_else(|| ControlError::not_found("no such job"))
    }

    // ------------------------------------------------------------- querying

    pub fn get(&self, id: &str) -> Option<JobSnapshot> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        reap_locked(&mut g);
        let job = g.jobs.get(id)?;
        Some(self.snapshot(job))
    }

    pub fn list(&self, f: &JobFilter) -> (Vec<JobSnapshot>, usize) {
        let (limit, offset) = f.clamped();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        reap_locked(&mut g);
        let mut all: Vec<&Job> = g.jobs.values().collect();
        all.sort_by_key(|j| j.seq);
        let filtered: Vec<&Job> = all
            .into_iter()
            .filter(|j| f.state.map(|s| j.state == s).unwrap_or(true))
            .filter(|j| f.op.map(|o| j.cmd.operation() == o).unwrap_or(true))
            .collect();
        let total = filtered.len();
        let page = filtered
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|j| self.snapshot(j))
            .collect();
        (page, total)
    }

    fn snapshot(&self, job: &Job) -> JobSnapshot {
        let artifacts = job
            .artifacts
            .iter()
            .filter_map(|a| self.artifacts.get(a))
            .map(|a| a.to_json())
            .collect();
        JobSnapshot {
            id: job.id.clone(),
            command: job.cmd.id.as_str().to_string(),
            corr: job.cmd.corr.clone(),
            operation: job.cmd.operation().as_str().to_string(),
            state: job.state.as_str().to_string(),
            created_ts: job.created_ts,
            updated_ts: job.updated_ts,
            deadline_ts: job.deadline_ts,
            resource: job.cmd.audit_target(),
            limits: job.cmd.limits,
            counters: job.counters,
            artifacts,
            result: job.result.clone(),
            error: job.error.clone(),
            cancel_requested: job.cancel.load(Ordering::Relaxed),
            events: job.events.clone(),
            audit_ref: format!("audit:{}", job.cmd.corr),
        }
    }

    /// In-flight jobs (queued + running), for the capacity banner.
    pub fn in_flight(&self) -> usize {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        reap_locked(&mut g);
        g.jobs.values().filter(|j| j.state.in_flight()).count()
    }

    pub fn running(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .jobs
            .values()
            .filter(|j| j.state == ExecState::Running)
            .count()
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .jobs
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Re-verify a delivery produced by an export job, or one registered
    /// artifact. Uses `harvest verify` semantics (no signing key: the control
    /// plane never holds one).
    pub fn verify(&self, job: Option<&str>, artifact: Option<&str>) -> Result<Value, ControlError> {
        let dir = match (job, artifact) {
            (Some(j), _) => {
                let snap = self.get(j).ok_or_else(|| {
                    ControlError::not_found("no such job (or it is no longer retained)")
                })?;
                if snap.operation != Operation::Export.as_str() {
                    return Err(ControlError::invalid(
                        "only export jobs produce deliveries that can be verified",
                    ));
                }
                if snap.state != ExecState::Completed.as_str() {
                    return Err(ControlError::conflict(format!(
                        "job {j} is {state}; only a completed export can be verified",
                        state = snap.state
                    )));
                }
                self.delivery_dir(j)
            }
            (None, Some(a)) => {
                let rec = self
                    .artifacts
                    .get(a)
                    .ok_or_else(|| ControlError::not_found("no such artifact"))?;
                let job_id = rec
                    .job
                    .clone()
                    .ok_or_else(|| ControlError::conflict("artifact is not part of a delivery"))?;
                self.delivery_dir(&job_id)
            }
            _ => return Err(ControlError::invalid("verify needs `job` or `artifact`")),
        };
        let dir = dir.ok_or_else(|| {
            ControlError::conflict("this job produced no delivery directory to verify")
        })?;
        let state = crate::export_job::verify_dir(&dir);
        // Re-verification is authoritative for the artifacts it covers: a
        // delivery whose manifest now fails stops being downloadable.
        let owner: Option<String> = match job {
            Some(j) => Some(j.to_string()),
            None => artifact.and_then(|a| self.artifacts.get(a).and_then(|r| r.job.clone())),
        };
        if let Some(job_id) = owner {
            for rec in self.artifacts.for_job(&job_id) {
                self.artifacts.mark_verified(&rec.id, state.clone());
            }
        }
        Ok(json!({
            "state": state.as_str(),
            "detail": state.to_json(),
            "delivery": dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
        }))
    }

    fn delivery_dir(&self, job_id: &str) -> Option<PathBuf> {
        let dir = self.root.join("deliveries").join(job_id);
        if dir.join("manifest.json").exists() {
            Some(dir)
        } else {
            None
        }
    }

    /// Artifact metadata (never bytes, never a path).
    pub fn inspect(&self, artifact_id: &str) -> Result<Value, ControlError> {
        let rec = self
            .artifacts
            .get(artifact_id)
            .ok_or_else(|| ControlError::not_found("no such artifact"))?;
        Ok(json!({
            "artifact": rec.to_json(),
            "job": rec.job.clone(),
            "verification": rec.verification.to_json(),
        }))
    }

    /// Evict the oldest terminal jobs past the retention cap.
    fn evict_locked(g: &mut RegistryInner) {
        if g.jobs.len() <= MAX_JOBS {
            return;
        }
        let mut victims: Vec<(u64, String)> = g
            .jobs
            .values()
            .filter(|j| j.state.is_terminal())
            .map(|j| (j.seq, j.id.clone()))
            .collect();
        victims.sort();
        for (_, id) in victims {
            if g.jobs.len() <= MAX_JOBS {
                break;
            }
            g.jobs.remove(&id);
        }
    }
}

fn push_event(job: &mut Job, kind: &str, detail: Value) {
    if job.events.len() >= MAX_EVENTS {
        job.events.remove(0);
    }
    job.events.push(JobEvent {
        ts: epoch_secs(),
        kind: kind.to_string(),
        detail,
    });
}

/// Deadline reaper: a job whose deadline has passed and which is somehow
/// still in flight is failed here. This is what guarantees no orphan: even a
/// task killed by the runtime cannot leave a job `running` forever.
fn reap_locked(g: &mut RegistryInner) {
    let now = epoch_secs();
    for job in g.jobs.values_mut() {
        if job.state.in_flight() && now > job.deadline_ts {
            job.state = ExecState::Failed;
            job.updated_ts = now;
            job.error = Some(format!(
                "job exceeded its deadline ({}s) and was reaped",
                job.deadline_ts.saturating_sub(job.created_ts)
            ));
            job.cancel.store(true, Ordering::Relaxed);
            push_event(job, "reaped", json!({ "deadline_ts": job.deadline_ts }));
        }
    }
}

#[cfg(test)]
mod tests;
