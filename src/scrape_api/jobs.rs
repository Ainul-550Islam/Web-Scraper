//! In-memory job registry with a durable snapshot.
//!
//! Jobs are the "do this later, possibly repeatedly" shape of the API. The
//! registry is the source of truth while the process is alive; the snapshot
//! on disk lets an operator inspect what was running and lets the API report
//! a job that an earlier process created. Cancellation is cooperative: the
//! record flips to `cancelled` and the worker checks it between runs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::ManagedApiCfg;
use crate::fsutil;
use crate::scrape_api::models::{ArtifactRef, ErrorBody, JobSchedule, JobState, ScrapeRequest};

/// One scheduled job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRecord {
    pub id: String,
    pub request: ScrapeRequest,
    #[serde(default)]
    pub schedule: JobSchedule,
    pub tenant_id: String,
    pub key_id: String,
    pub key_hash: String,
    pub state: JobState,
    pub request_id: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub started_at_ms: Option<u64>,
    #[serde(default)]
    pub finished_at_ms: Option<u64>,
    /// The finished scrape, as JSON, once the job has run.
    #[serde(default)]
    pub response: Option<crate::scrape_api::models::ScrapeResponse>,
    #[serde(default)]
    pub records: Vec<serde_json::Value>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
    #[serde(default)]
    pub attempts: u8,
    #[serde(default)]
    pub runs_completed: u32,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

impl JobRecord {
    /// A job is done when it reaches a terminal state, or when it has run
    /// its repeat count, or when it was cancelled.
    pub fn exhausted(&self) -> bool {
        if self.cancelled {
            return true;
        }
        self.state.terminal()
            || (self.schedule.repeat != 0 && self.runs_completed >= self.schedule.repeat)
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    #[serde(default)]
    jobs: BTreeMap<String, JobRecord>,
}

/// The registry. Guarded by a single mutex: job mutation is rare and the
/// critical section is short.
pub struct JobRegistry {
    cfg: ManagedApiCfg,
    dir: PathBuf,
    path: PathBuf,
    lock: Mutex<BTreeMap<String, JobRecord>>,
}

impl JobRegistry {
    pub fn open(cfg: &ManagedApiCfg, dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        fsutil::prepare_private_directory(dir)
            .with_context(|| format!("cannot prepare job directory {}", dir.display()))?;
        let path = dir.join("jobs.json");
        let mut jobs = BTreeMap::new();
        if path.exists() {
            let text = fsutil::read_bounded(&path, 8 * 1024 * 1024)
                .with_context(|| format!("cannot read job snapshot {}", path.display()))?;
            let snap: Snapshot = serde_json::from_slice(&text)?;
            // Jobs that were running when the process died are failed, not
            // silently resumed: a resumed job could double-charge a tenant.
            for (_, job) in snap.jobs {
                let mut job = job;
                if job.state == JobState::Running {
                    job.state = JobState::Failed;
                    job.error = Some(ErrorBody::new(
                        "job_interrupted",
                        "the job did not finish before the process stopped",
                    ));
                    job.finished_at_ms = Some(crate::scrape_api::time_now_ms());
                }
                jobs.insert(job.id.clone(), job);
            }
        }
        Ok(Self {
            cfg: cfg.clone(),
            dir: dir.to_path_buf(),
            path,
            lock: Mutex::new(jobs),
        })
    }

    /// Insert a new job, evicting the oldest if the registry is full.
    pub fn create(&self, job: JobRecord) -> Result<()> {
        let mut guard = self.lock.lock().unwrap();
        if guard.len() >= self.cfg.jobs.max_jobs {
            if let Some(oldest) = guard.keys().next().cloned() {
                guard.remove(&oldest);
            }
        }
        guard.insert(job.id.clone(), job);
        let body = serde_json::to_string(&Snapshot {
            jobs: guard.clone(),
        })?;
        self.persist(&body)?;
        Ok(())
    }

    fn persist(&self, body: &str) -> Result<()> {
        fsutil::prepare_private_directory(&self.dir)?;
        fsutil::write_private_atomic(&self.path, body.as_bytes())?;
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<JobRecord> {
        self.lock.lock().unwrap().get(id).cloned()
    }

    /// Mutate a job and persist the result. Returns `None` when the id is
    /// unknown.
    pub fn update<F>(&self, id: &str, f: F) -> Option<JobRecord>
    where
        F: FnOnce(&mut JobRecord),
    {
        let mut guard = self.lock.lock().unwrap();
        let job = guard.get_mut(id)?;
        f(job);
        let updated = job.clone();
        let body = serde_json::to_string(&Snapshot {
            jobs: guard.clone(),
        })
        .ok();
        drop(guard);
        if let Some(body) = body {
            let _ = self.persist(&body);
        }
        Some(updated)
    }

    /// Atomically test-and-set the state, so a concurrent cancel cannot
    /// interleave with a worker's completion.
    pub fn transition(
        &self,
        id: &str,
        expected: JobState,
        next: JobState,
        f: impl FnOnce(&mut JobRecord),
    ) -> Option<JobRecord> {
        let mut guard = self.lock.lock().unwrap();
        let job = guard.get_mut(id)?;
        if job.state != expected {
            return None;
        }
        job.state = next;
        f(job);
        let updated = job.clone();
        let body = serde_json::to_string(&Snapshot {
            jobs: guard.clone(),
        })
        .ok();
        drop(guard);
        if let Some(body) = body {
            let _ = self.persist(&body);
        }
        Some(updated)
    }

    pub fn cancel(&self, id: &str) -> Option<JobRecord> {
        let id_owned = id.to_string();
        self.update(&id_owned, |job| {
            if job.state.terminal() {
                return;
            }
            job.cancelled = true;
            job.state = JobState::Cancelled;
            job.finished_at_ms = Some(crate::scrape_api::time_now_ms());
        })
    }

    /// Jobs that still need to run, oldest first.
    pub fn pending(&self) -> Vec<JobRecord> {
        let guard = self.lock.lock().unwrap();
        guard
            .values()
            .filter(|j| !j.exhausted() && !j.cancelled)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.lock.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
