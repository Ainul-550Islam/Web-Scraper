//! Durable, atomic JSON repository for browser jobs, metadata, workflows,
//! observations, artifact references and provider attempts. One process owns a
//! repository instance; private atomic replacement prevents partial snapshots.
use crate::browser::api_models::{BrowserJobDto, BrowserPreviewRequest, BrowserSessionDto};
use crate::browser::error::{BrowserError, BrowserResult};
use crate::browser::page::{PageArtifactRef, PageObservation};
use crate::browser::workflow::Workflow;
use crate::selector::ast::SelectorTree;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_OBSERVATIONS: usize = 10_000;
const MAX_PROVIDER_ATTEMPTS: usize = 10_000;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredWorkflow {
    pub id: String,
    pub version: u64,
    pub workflow: Workflow,
    pub selector_tree: Option<SelectorTree>,
    pub updated_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderAttempt {
    pub request_id: String,
    pub provider_id: String,
    pub target_id: String,
    pub domain: String,
    pub status: String,
    pub cost_microusd: u64,
    pub elapsed_ms: u64,
    pub created_at_ms: u64,
    pub safe_message: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    pub jobs: BTreeMap<String, BrowserJobDto>,
    pub job_requests: BTreeMap<String, BrowserPreviewRequest>,
    pub sessions: BTreeMap<String, BrowserSessionDto>,
    pub workflows: BTreeMap<String, StoredWorkflow>,
    pub observations: Vec<PageObservation>,
    pub artifacts: BTreeMap<String, PageArtifactRef>,
    pub provider_attempts: Vec<ProviderAttempt>,
}
#[derive(Serialize, Deserialize)]
struct RepositoryFile {
    version: u32,
    snapshot: RepositorySnapshot,
}
pub struct BrowserRepository {
    path: PathBuf,
    lock: Mutex<()>,
}
impl BrowserRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }
    pub fn snapshot(&self) -> BrowserResult<RepositorySnapshot> {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        self.read_unlocked()
    }
    pub fn save_job(&self, job: BrowserJobDto) -> BrowserResult<()> {
        validate_id(&job.id)?;
        self.update(|s| {
            s.jobs.insert(job.id.clone(), job);
            Ok(())
        })
    }
    pub fn save_job_request(&self, id: &str, request: BrowserPreviewRequest) -> BrowserResult<()> {
        validate_id(id)?;
        self.update(|s| {
            s.job_requests.insert(id.to_owned(), request);
            Ok(())
        })
    }
    pub fn save_job_bundle(
        &self,
        job: BrowserJobDto,
        request: BrowserPreviewRequest,
    ) -> BrowserResult<()> {
        validate_id(&job.id)?;
        self.update(|s| {
            s.job_requests.insert(job.id.clone(), request);
            s.jobs.insert(job.id.clone(), job);
            Ok(())
        })
    }
    pub fn remove_job_request(&self, id: &str) -> BrowserResult<bool> {
        let mut removed = false;
        self.update(|s| {
            removed = s.job_requests.remove(id).is_some();
            Ok(())
        })?;
        Ok(removed)
    }
    pub fn save_session(&self, session: BrowserSessionDto) -> BrowserResult<()> {
        validate_id(&session.id)?;
        self.update(|s| {
            s.sessions.insert(session.id.clone(), session);
            Ok(())
        })
    }
    pub fn remove_session(&self, id: &str) -> BrowserResult<bool> {
        validate_id(id)?;
        let mut removed = false;
        self.update(|s| {
            removed = s.sessions.remove(id).is_some();
            Ok(())
        })?;
        Ok(removed)
    }
    pub fn save_workflow(
        &self,
        id: &str,
        expected_version: u64,
        workflow: Workflow,
        selector_tree: Option<SelectorTree>,
    ) -> BrowserResult<StoredWorkflow> {
        validate_id(id)?;
        workflow
            .validate_persistable()
            .map_err(|e| BrowserError::Config(e.to_string()))?;
        if let Some(tree) = &selector_tree {
            tree.validate()
                .map_err(|e| BrowserError::Config(e.to_string()))?;
        }
        let mut saved = None;
        self.update(|s| {
            let current = s.workflows.get(id).map(|w| w.version).unwrap_or(0);
            if current != expected_version {
                return Err(BrowserError::Policy("workflow version conflict".into()));
            }
            let value = StoredWorkflow {
                id: id.into(),
                version: current.saturating_add(1),
                workflow,
                selector_tree,
                updated_at_ms: crate::browser::page::now_ms(),
            };
            s.workflows.insert(id.into(), value.clone());
            saved = Some(value);
            Ok(())
        })?;
        saved.ok_or(BrowserError::InvalidInput("workflow save"))
    }
    pub fn workflow(&self, id: &str) -> BrowserResult<Option<StoredWorkflow>> {
        Ok(self.snapshot()?.workflows.remove(id))
    }
    pub fn append_observation(&self, observation: PageObservation) -> BrowserResult<()> {
        self.update(|s| {
            if s.observations.len() >= MAX_OBSERVATIONS {
                s.observations.remove(0);
            }
            s.observations.push(observation);
            Ok(())
        })
    }
    pub fn save_artifact_ref(&self, artifact: PageArtifactRef) -> BrowserResult<()> {
        validate_id(&artifact.id)?;
        self.update(|s| {
            s.artifacts.insert(artifact.id.clone(), artifact);
            Ok(())
        })
    }
    pub fn append_provider_attempt(&self, attempt: ProviderAttempt) -> BrowserResult<()> {
        validate_id(&attempt.request_id)?;
        self.update(|s| {
            if s.provider_attempts.len() >= MAX_PROVIDER_ATTEMPTS {
                s.provider_attempts.remove(0);
            }
            s.provider_attempts.push(attempt);
            Ok(())
        })
    }
    fn update<F>(&self, f: F) -> BrowserResult<()>
    where
        F: FnOnce(&mut RepositorySnapshot) -> BrowserResult<()>,
    {
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut snapshot = self.read_unlocked()?;
        f(&mut snapshot)?;
        let bytes = serde_json::to_vec(&RepositoryFile {
            version: VERSION,
            snapshot,
        })
        .map_err(|_| BrowserError::InvalidInput("browser repository serialization"))?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(BrowserError::ContentLimit {
                kind: "browser repository",
                limit: MAX_BYTES as usize,
            });
        }
        crate::fsutil::write_private_atomic(&self.path, &bytes).map_err(|_| {
            BrowserError::Policy("browser repository could not be safely written".into())
        })
    }
    fn read_unlocked(&self) -> BrowserResult<RepositorySnapshot> {
        let bytes = match crate::fsutil::read_bounded(&self.path, MAX_BYTES) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RepositorySnapshot::default())
            }
            Err(_) => {
                return Err(BrowserError::Policy(
                    "browser repository could not be safely read".into(),
                ))
            }
        };
        let file: RepositoryFile = serde_json::from_slice(&bytes)
            .map_err(|_| BrowserError::InvalidInput("browser repository format"))?;
        if file.version != VERSION {
            return Err(BrowserError::Unsupported("browser repository version"));
        }
        Ok(file.snapshot)
    }
}
fn validate_id(id: &str) -> BrowserResult<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(BrowserError::InvalidInput("repository id"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_workflow_with_optimistic_version() {
        let path =
            std::env::temp_dir().join(format!("harvest-browser-repo-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let repo = BrowserRepository::new(&path);
        let w = Workflow::default();
        let saved = repo.save_workflow("flow", 0, w.clone(), None).unwrap();
        assert_eq!(saved.version, 1);
        assert!(repo.save_workflow("flow", 0, w, None).is_err());
        let _ = std::fs::remove_file(path);
    }
}
