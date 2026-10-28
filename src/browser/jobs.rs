//! Durable, idempotent browser execution queue with leases, heartbeat,
//! cancellation, bounded retries and terminal-state persistence.
use crate::browser::api_models::{BrowserJobDto, BrowserPreviewRequest};
use crate::browser::error::{BrowserError, BrowserResult};
use crate::browser::page::now_ms;
use crate::browser::repository::BrowserRepository;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const MAX_QUEUE: usize = 10_000;
struct QueuedJob {
    dto: BrowserJobDto,
    request: BrowserPreviewRequest,
    worker: Option<String>,
    lease_until_ms: Option<u64>,
    cancel_requested: bool,
}
#[derive(Default)]
struct QueueState {
    jobs: BTreeMap<String, QueuedJob>,
    sequence: u64,
}
#[derive(Clone, Debug)]
pub struct BrowserJobLease {
    pub job: BrowserJobDto,
    pub request: BrowserPreviewRequest,
    pub worker_id: String,
    pub lease_until_ms: u64,
}

pub struct BrowserJobQueue {
    repository: Arc<BrowserRepository>,
    capacity: usize,
    max_attempts: u32,
    lease_ms: u64,
    state: Mutex<QueueState>,
}
impl BrowserJobQueue {
    pub fn open(
        repository: Arc<BrowserRepository>,
        capacity: usize,
        max_attempts: u32,
        lease_ms: u64,
    ) -> BrowserResult<Self> {
        if capacity == 0
            || capacity > MAX_QUEUE
            || !(1..=16).contains(&max_attempts)
            || !(1000..=3_600_000).contains(&lease_ms)
        {
            return Err(BrowserError::InvalidInput("browser job queue bounds"));
        }
        let snapshot = repository.snapshot()?;
        let mut jobs = BTreeMap::new();
        for (id, dto) in snapshot.jobs {
            if let Some(request) = snapshot.job_requests.get(&id) {
                let dto = if dto.state == "running" || dto.state == "cancelling" {
                    let mut recovered = dto.clone();
                    recovered.state = "queued".into();
                    recovered.started_at_ms = None;
                    recovered.error =
                        Some("recovered after process restart; lease released".into());
                    recovered
                } else {
                    dto
                };
                jobs.insert(
                    id,
                    QueuedJob {
                        dto,
                        request: request.clone(),
                        worker: None,
                        lease_until_ms: None,
                        cancel_requested: false,
                    },
                );
            }
        }
        Ok(Self {
            repository,
            capacity,
            max_attempts,
            lease_ms,
            state: Mutex::new(QueueState { jobs, sequence: 0 }),
        })
    }

    pub fn enqueue(
        &self,
        request: BrowserPreviewRequest,
        idempotency_key: &str,
    ) -> BrowserResult<BrowserJobDto> {
        validate_idempotency(idempotency_key)?;
        validate_request(&request)?;
        let body = serde_json::to_vec(&request)
            .map_err(|_| BrowserError::InvalidInput("browser job payload"))?;
        let digest = hex(&Sha256::digest(&body));
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = state
            .jobs
            .values()
            .find(|j| j.dto.idempotency_key == idempotency_key)
        {
            if existing.dto.body_digest != digest {
                return Err(BrowserError::Policy(
                    "idempotency key was already used with a different browser request".into(),
                ));
            }
            return Ok(existing.dto.clone());
        }
        if state
            .jobs
            .values()
            .filter(|j| !terminal(&j.dto.state))
            .count()
            >= self.capacity
        {
            return Err(BrowserError::Policy("browser job queue is full".into()));
        }
        state.sequence = state.sequence.saturating_add(1);
        let now = now_ms();
        let id = format!("bj-{}-{}", now, state.sequence);
        let dto = BrowserJobDto {
            id: id.clone(),
            target_id: request.target_id.clone(),
            state: "queued".into(),
            idempotency_key: idempotency_key.into(),
            body_digest: digest,
            attempts: 0,
            queued_at_ms: now,
            started_at_ms: None,
            finished_at_ms: None,
            error: None,
            artifact_ids: Vec::new(),
        };
        self.repository
            .save_job_bundle(dto.clone(), request.clone())?;
        state.jobs.insert(
            id,
            QueuedJob {
                dto: dto.clone(),
                request,
                worker: None,
                lease_until_ms: None,
                cancel_requested: false,
            },
        );
        Ok(dto)
    }

    pub fn claim(&self, worker_id: &str) -> BrowserResult<Option<BrowserJobLease>> {
        validate_worker(worker_id)?;
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let now = now_ms();
        for job in state.jobs.values_mut() {
            if job.dto.state == "running" && job.lease_until_ms.is_some_and(|until| until <= now) {
                job.worker = None;
                job.lease_until_ms = None;
                if job.dto.attempts >= self.max_attempts {
                    job.dto.state = "failed".into();
                    job.dto.error = Some("browser job lease expired after retry limit".into());
                    job.dto.finished_at_ms = Some(now);
                } else {
                    job.dto.state = "queued".into();
                    job.dto.error = Some("browser job lease expired; requeued".into());
                }
            }
        }
        let Some(job) = state.jobs.values_mut().find(|j| j.dto.state == "queued") else {
            return Ok(None);
        };
        job.dto.state = "running".into();
        job.dto.attempts = job.dto.attempts.saturating_add(1);
        job.dto.started_at_ms = Some(now);
        job.dto.error = None;
        job.worker = Some(worker_id.into());
        job.lease_until_ms = Some(now.saturating_add(self.lease_ms));
        let lease = BrowserJobLease {
            job: job.dto.clone(),
            request: job.request.clone(),
            worker_id: worker_id.into(),
            lease_until_ms: job.lease_until_ms.unwrap_or(now),
        };
        self.repository.save_job(lease.job.clone())?;
        Ok(Some(lease))
    }

    pub fn heartbeat(&self, id: &str, worker: &str) -> BrowserResult<u64> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let job = state
            .jobs
            .get_mut(id)
            .ok_or(BrowserError::NotFound("browser job"))?;
        verify_lease(job, worker)?;
        let until = now_ms().saturating_add(self.lease_ms);
        job.lease_until_ms = Some(until);
        self.repository.save_job(job.dto.clone())?;
        Ok(until)
    }
    pub fn is_cancel_requested(&self, id: &str, worker: &str) -> BrowserResult<bool> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let job = state
            .jobs
            .get(id)
            .ok_or(BrowserError::NotFound("browser job"))?;
        verify_lease(job, worker)?;
        Ok(job.cancel_requested)
    }
    pub fn cancel(&self, id: &str) -> BrowserResult<BrowserJobDto> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let job = state
            .jobs
            .get_mut(id)
            .ok_or(BrowserError::NotFound("browser job"))?;
        match job.dto.state.as_str() {
            "queued" => {
                job.dto.state = "cancelled".into();
                job.dto.finished_at_ms = Some(now_ms());
            }
            "running" => {
                job.dto.state = "cancelling".into();
                job.cancel_requested = true;
            }
            "cancelling" | "cancelled" | "completed" | "failed" => {}
            _ => return Err(BrowserError::InvalidInput("job state")),
        }
        self.repository.save_job(job.dto.clone())?;
        Ok(job.dto.clone())
    }
    pub fn complete(
        &self,
        id: &str,
        worker: &str,
        artifact_ids: Vec<String>,
    ) -> BrowserResult<BrowserJobDto> {
        if artifact_ids.len() > 256 || artifact_ids.iter().any(|s| s.is_empty() || s.len() > 128) {
            return Err(BrowserError::InvalidInput("job artifact references"));
        }
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let job = state
            .jobs
            .get_mut(id)
            .ok_or(BrowserError::NotFound("browser job"))?;
        verify_lease(job, worker)?;
        job.dto.state = if job.cancel_requested {
            "cancelled".into()
        } else {
            "completed".into()
        };
        job.dto.artifact_ids = artifact_ids;
        job.dto.finished_at_ms = Some(now_ms());
        job.worker = None;
        job.lease_until_ms = None;
        self.repository.save_job(job.dto.clone())?;
        Ok(job.dto.clone())
    }
    pub fn fail(
        &self,
        id: &str,
        worker: &str,
        error: &BrowserError,
        retryable: bool,
    ) -> BrowserResult<BrowserJobDto> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let job = state
            .jobs
            .get_mut(id)
            .ok_or(BrowserError::NotFound("browser job"))?;
        verify_lease(job, worker)?;
        let safe = error.to_string().chars().take(256).collect::<String>();
        if retryable && job.dto.attempts < self.max_attempts && !job.cancel_requested {
            job.dto.state = "queued".into();
            job.dto.error = Some(safe);
        } else {
            job.dto.state = if job.cancel_requested {
                "cancelled".into()
            } else {
                "failed".into()
            };
            job.dto.error = Some(safe);
            job.dto.finished_at_ms = Some(now_ms());
        }
        job.worker = None;
        job.lease_until_ms = None;
        self.repository.save_job(job.dto.clone())?;
        Ok(job.dto.clone())
    }
    pub fn get(&self, id: &str) -> Option<BrowserJobDto> {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .jobs
            .get(id)
            .map(|j| j.dto.clone())
    }
    pub fn list(&self, limit: usize) -> Vec<BrowserJobDto> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state
            .jobs
            .values()
            .rev()
            .take(limit.min(1000))
            .map(|j| j.dto.clone())
            .collect()
    }
}

fn validate_request(r: &BrowserPreviewRequest) -> BrowserResult<()> {
    if r.target_id.is_empty()
        || r.target_id.len() > 128
        || r.request_id.is_empty()
        || r.request_id.len() > 128
    {
        return Err(BrowserError::InvalidInput("browser job id"));
    }
    let u = url::Url::parse(&r.target_url)
        .map_err(|_| BrowserError::InvalidInput("browser target URL"))?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return Err(BrowserError::Policy(
            "browser job URL must be credential-free http(s)".into(),
        ));
    }
    if r.allowed_hosts.len() > 128 {
        return Err(BrowserError::ContentLimit {
            kind: "allowed hosts",
            limit: 128,
        });
    }
    r.workflow
        .validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    if let Some(t) = &r.selector_tree {
        t.validate()
            .map_err(|e| BrowserError::Config(e.to_string()))?;
    }
    Ok(())
}
fn validate_idempotency(s: &str) -> BrowserResult<()> {
    if s.is_empty()
        || s.len() > 64
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        Err(BrowserError::InvalidInput("idempotency key"))
    } else {
        Ok(())
    }
}
fn validate_worker(s: &str) -> BrowserResult<()> {
    if s.is_empty()
        || s.len() > 128
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(BrowserError::InvalidInput("worker id"))
    } else {
        Ok(())
    }
}
fn verify_lease(job: &QueuedJob, worker: &str) -> BrowserResult<()> {
    if job.worker.as_deref() != Some(worker)
        || job.lease_until_ms.is_none_or(|until| until <= now_ms())
    {
        Err(BrowserError::Policy(
            "browser job lease is stale or owned by another worker".into(),
        ))
    } else {
        Ok(())
    }
}
fn terminal(s: &str) -> bool {
    matches!(s, "completed" | "failed" | "cancelled")
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::repository::BrowserRepository;
    use crate::browser::workflow::Workflow;
    fn request() -> BrowserPreviewRequest {
        BrowserPreviewRequest {
            request_id: "r1".into(),
            target_id: "t1".into(),
            target_url: "https://example.test".into(),
            session_id: None,
            allowed_hosts: vec!["example.test".into()],
            workflow: Workflow::default(),
            selector_tree: None,
            capture_screenshot: false,
        }
    }
    #[test]
    fn idempotency_is_body_bound_and_jobs_are_leased() {
        let path =
            std::env::temp_dir().join(format!("harvest-browser-jobs-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let repo = Arc::new(BrowserRepository::new(&path));
        let q = BrowserJobQueue::open(repo, 4, 2, 5000).unwrap();
        let first = q.enqueue(request(), "key-1").unwrap();
        let same = q.enqueue(request(), "key-1").unwrap();
        assert_eq!(first.id, same.id);
        let lease = q.claim("worker-1").unwrap().unwrap();
        assert_eq!(lease.job.attempts, 1);
        assert!(q.heartbeat(&first.id, "worker-2").is_err());
        assert!(q.complete(&first.id, "worker-1", vec![]).unwrap().state == "completed");
        let _ = std::fs::remove_file(path);
    }
}
