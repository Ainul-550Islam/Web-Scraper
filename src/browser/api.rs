//! Browser preview/control service. HTTP routing and authentication are owned
//! by `admin`; this service contains the feature-gated runtime operations.
use super::api_models::BrowserJobDto;
use super::api_models::*;
use super::artifacts::BrowserArtifactStore;
use super::config::BrowserConfig;
use super::error::{BrowserError, BrowserResult};
use super::jobs::{BrowserJobLease, BrowserJobQueue};
use super::page::PageObservation;
use super::repository::{BrowserRepository, StoredWorkflow};
#[cfg(feature = "browser")]
use std::collections::{BTreeMap, HashMap};
#[cfg(feature = "browser")]
use super::credentials::EnvironmentCredentialResolver;
#[cfg(feature = "browser")]
use super::interaction;
#[cfg(feature = "browser")]
use super::navigation::{self, WaitUntil};
#[cfg(feature = "browser")]
use super::page::PageArtifactRef;
#[cfg(feature = "browser")]
use super::session::BrowserSession;
#[cfg(feature = "browser")]
use super::workflow::{InputValue, Workflow, WorkflowStep};
#[cfg(feature = "browser")]
use crate::selector::engine;
#[cfg(feature = "browser")]
use serde_json::json;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[cfg(feature = "browser")]
#[derive(Clone)]
struct SessionScope {
    runtime_key: String,
    target_id: String,
    allowed_hosts: Vec<String>,
    created_at_ms: u64,
}

pub struct BrowserApi {
    config: BrowserConfig,
    repository: Arc<BrowserRepository>,
    artifacts: Arc<BrowserArtifactStore>,
    jobs: Arc<BrowserJobQueue>,
    #[cfg(feature = "browser")]
    scopes: Mutex<HashMap<String, SessionScope>>,
    #[cfg(feature = "browser")]
    targets: HashMap<String, crate::config::TargetCfg>,
    #[cfg(feature = "browser")]
    runtime: Mutex<Option<super::runtime::BrowserRuntime>>,
    startup: tokio::sync::Mutex<()>,
    #[cfg(feature = "browser")]
    last_error: Mutex<Option<String>>,
}
impl BrowserApi {
    pub fn new(
        config: BrowserConfig,
        repository: Arc<BrowserRepository>,
        artifact_root: impl Into<PathBuf>,
    ) -> BrowserResult<Self> {
        config
            .validate()
            .map_err(|e| BrowserError::Config(e.to_string()))?;
        let jobs = Arc::new(BrowserJobQueue::open(repository.clone(), 128, 3, 30_000)?);
        Ok(Self {
            config,
            repository,
            artifacts: Arc::new(BrowserArtifactStore::new(artifact_root)),
            jobs,
            #[cfg(feature = "browser")]
            scopes: Mutex::new(HashMap::new()),
            #[cfg(feature = "browser")]
            targets: HashMap::new(),
            #[cfg(feature = "browser")]
            runtime: Mutex::new(None),
            startup: tokio::sync::Mutex::new(()),
            #[cfg(feature = "browser")]
            last_error: Mutex::new(None),
        })
    }
    #[cfg(feature = "browser")]
    pub fn with_targets(mut self, targets: Vec<crate::config::TargetCfg>) -> Self {
        self.targets = targets.into_iter().map(|target| (target.id.clone(), target)).collect();
        self
    }
    #[cfg(feature = "browser")]
    pub async fn launch(
        config: BrowserConfig,
        repository: Arc<BrowserRepository>,
        artifact_root: impl Into<PathBuf>,
    ) -> BrowserResult<Self> {
        let api = Self::new(config, repository, artifact_root)?;
        if api.config.enabled { api.ensure_runtime().await?; }
        Ok(api)
    }
    #[cfg(feature = "browser")]
    pub fn attach_runtime(&self, runtime: super::runtime::BrowserRuntime) {
        *self.runtime.lock().unwrap_or_else(|p| p.into_inner()) = Some(runtime);
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    #[cfg(feature = "browser")]
    pub fn set_start_error(&self, error: &BrowserError) {
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some(error.to_string());
    }
    #[cfg(feature = "browser")]
    fn runtime(&self) -> BrowserResult<super::runtime::BrowserRuntime> {
        self.runtime
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .ok_or(BrowserError::Policy(
                "browser runtime is not attached".into(),
            ))
    }
    #[cfg(feature = "browser")]
    async fn ensure_runtime(&self) -> BrowserResult<super::runtime::BrowserRuntime> {
        if let Ok(runtime)=self.runtime(){return Ok(runtime);}
        if !self.config.enabled{return Err(BrowserError::Policy("browser support is disabled by configuration".into()));}
        let _guard=self.startup.lock().await;
        if let Ok(runtime)=self.runtime(){return Ok(runtime);}
        match super::runtime::BrowserRuntime::launch(self.config.clone()).await {
            Ok(runtime)=>{self.attach_runtime(runtime.clone());Ok(runtime)}
            Err(error)=>{self.set_start_error(&error);Err(error)}
        }
    }

    pub fn diagnostics(&self) -> BrowserDiagnostics {
        #[cfg(feature = "browser")]
        {
            let runtime = self.runtime.lock().unwrap_or_else(|p| p.into_inner());
            let active = runtime.as_ref().map(|r| r.active_sessions()).unwrap_or(0);
            return BrowserDiagnostics {
                enabled: self.config.enabled,
                feature_compiled: true,
                runtime_ready: runtime.is_some(),
                active_sessions: active,
                max_sessions: self.config.max_sessions,
                engine: format!("{:?}", self.config.engine).to_ascii_lowercase(),
                last_error: self
                    .last_error
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone(),
            };
        }
        #[cfg(not(feature = "browser"))]
        {
            BrowserDiagnostics {
                enabled: self.config.enabled,
                feature_compiled: false,
                runtime_ready: false,
                active_sessions: 0,
                max_sessions: self.config.max_sessions,
                engine: format!("{:?}", self.config.engine).to_ascii_lowercase(),
                last_error: if self.config.enabled {
                    Some("binary lacks --features browser".into())
                } else {
                    None
                },
            }
        }
    }

    #[cfg(feature = "browser")]
    pub async fn preview(
        &self,
        request: BrowserPreviewRequest,
    ) -> BrowserResult<BrowserPreviewResponse> {
        validate_preview(&request)?;
        let runtime = self.ensure_runtime().await?;
        let parsed = url::Url::parse(&request.target_url)
            .map_err(|_| BrowserError::InvalidInput("preview URL"))?;
        let host = parsed
            .host_str()
            .ok_or(BrowserError::InvalidInput("preview URL host"))?
            .to_ascii_lowercase();
        let (external_id, runtime_key, allowed_hosts) = self.resolve_scope(&request, &host)?;
        let session = runtime.session(&runtime_key, None, None).await?;
        let _ = navigation::navigate(
            &session,
            &request.target_url,
            WaitUntil::Load,
            self.config.navigation_timeout_ms,
        )
        .await?;
        let resolver = EnvironmentCredentialResolver;
        interaction::run_workflow_scoped(
            &session,
            &self.config,
            &request.workflow,
            &resolver,
            &allowed_hosts,
        )
        .await?;
        let mut page = runtime.observe(&session, &request.target_url).await?;
        let challenge = super::challenge::detect(&page);
        let mut artifacts = Vec::new();
        if self.config.capture_dom {
            if let Some(html) = page.html.as_deref() {
                let r = super::artifacts::store_dom(
                    &self.artifacts,
                    &session.id(),
                    html,
                    self.config.max_html_bytes,
                )?;
                page.add_artifact(r.clone(), 8);
                artifacts.push(r.clone());
                self.repository.save_artifact_ref(r)?;
            }
        }
        if request.capture_screenshot
            && self.config.capture_screenshot_on_failure
            && !challenge.detected
        { /* failure-only policy does not capture successful pages */
        }
        if request.capture_screenshot {
            let r = super::artifacts::capture_screenshot(
                &session,
                &self.artifacts,
                self.config.max_screenshot_bytes,
            )
            .await?;
            page.add_artifact(r.clone(), 8);
            artifacts.push(r.clone());
            self.repository.save_artifact_ref(r)?;
        }
        let records = if let Some(tree) = &request.selector_tree {
            engine::extract(page.html.as_deref().unwrap_or(""), tree, 10_000)?
        } else {
            Vec::new()
        };
        page.finish();
        self.repository.append_observation(page.clone())?;
        self.persist_session(&external_id, &runtime_key, &session, None)?;
        Ok(BrowserPreviewResponse {
            request_id: request.request_id,
            page,
            challenge,
            records,
            artifacts,
        })
    }

    #[cfg(not(feature = "browser"))]
    pub async fn preview(
        &self,
        _request: BrowserPreviewRequest,
    ) -> BrowserResult<BrowserPreviewResponse> {
        Err(BrowserError::Unsupported(
            "browser runtime; compile with --features browser",
        ))
    }

    #[cfg(feature = "browser")]
    pub async fn navigate(
        &self,
        request: BrowserNavigateRequest,
    ) -> BrowserResult<PageObservation> {
        let session = self.get_session(&request.session_id).await?;
        let host = url::Url::parse(&request.url)
            .map_err(|_| BrowserError::InvalidInput("navigation URL"))?
            .host_str()
            .ok_or(BrowserError::InvalidInput("navigation URL host"))?
            .to_ascii_lowercase();
        let scope = self.scope_for(&request.session_id)?;
        if !scope
            .allowed_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(&host))
        {
            return Err(BrowserError::Policy(
                "navigation is outside the stored session scope".into(),
            ));
        }
        let _ = navigation::navigate(
            &session,
            &request.url,
            WaitUntil::Load,
            self.config.navigation_timeout_ms,
        )
        .await?;
        let page = self.runtime()?.observe(&session, &request.url).await?;
        self.repository.append_observation(page.clone())?;
        Ok(page)
    }
    #[cfg(not(feature = "browser"))]
    pub async fn navigate(
        &self,
        _request: BrowserNavigateRequest,
    ) -> BrowserResult<PageObservation> {
        Err(BrowserError::Unsupported("browser runtime"))
    }

    #[cfg(feature = "browser")]
    pub async fn inspect(
        &self,
        request: BrowserInspectRequest,
    ) -> BrowserResult<BrowserPreviewResponse> {
        let session = self.get_session(&request.session_id).await?;
        let url = session.current_url().await?;
        let mut page = self.runtime()?.observe(&session, &url).await?;
        let challenge = super::challenge::detect(&page);
        let mut artifacts = Vec::new();
        if request.capture_dom {
            if let Some(html) = page.html.as_deref() {
                let a = super::artifacts::store_dom(
                    &self.artifacts,
                    session.id(),
                    html,
                    self.config.max_html_bytes,
                )?;
                page.add_artifact(a.clone(), 8);
                artifacts.push(a.clone());
                self.repository.save_artifact_ref(a)?;
            }
        }
        if request.capture_screenshot {
            let a = super::artifacts::capture_screenshot(
                &session,
                &self.artifacts,
                self.config.max_screenshot_bytes,
            )
            .await?;
            page.add_artifact(a.clone(), 8);
            artifacts.push(a.clone());
            self.repository.save_artifact_ref(a)?;
        }
        let records = Vec::new();
        self.repository.append_observation(page.clone())?;
        Ok(BrowserPreviewResponse {
            request_id: format!("inspect-{}", page.page_id),
            page,
            challenge,
            records,
            artifacts,
        })
    }
    #[cfg(not(feature = "browser"))]
    pub async fn inspect(
        &self,
        _request: BrowserInspectRequest,
    ) -> BrowserResult<BrowserPreviewResponse> {
        Err(BrowserError::Unsupported("browser runtime"))
    }

    #[cfg(feature = "browser")]
    pub async fn test_selector(
        &self,
        request: SelectorTestRequest,
    ) -> BrowserResult<Vec<crate::selector::record::Record>> {
        if !(1..=10_000).contains(&request.max_records) {
            return Err(BrowserError::InvalidInput("selector record limit"));
        }
        let session = self.get_session(&request.session_id).await?;
        let html = session.page_source(self.config.max_html_bytes).await?;
        engine::extract(&html, &request.selector_tree, request.max_records)
    }
    #[cfg(not(feature = "browser"))]
    pub async fn test_selector(
        &self,
        _request: SelectorTestRequest,
    ) -> BrowserResult<Vec<crate::selector::record::Record>> {
        Err(BrowserError::Unsupported("browser runtime"))
    }

    #[cfg(feature = "browser")]
    pub async fn run_action(
        &self,
        request: BrowserActionRequest,
    ) -> BrowserResult<PageObservation> {
        let session = self.get_session(&request.session_id).await?;
        let scope = self.scope_for(&request.session_id)?;
        let resolver = EnvironmentCredentialResolver;
        interaction::run_workflow_scoped(
            &session,
            &self.config,
            &request.action,
            &resolver,
            &scope.allowed_hosts,
        )
        .await?;
        let url = session.current_url().await?;
        let page = self.runtime()?.observe(&session, &url).await?;
        self.repository.append_observation(page.clone())?;
        Ok(page)
    }
    #[cfg(not(feature = "browser"))]
    pub async fn run_action(
        &self,
        _request: BrowserActionRequest,
    ) -> BrowserResult<PageObservation> {
        Err(BrowserError::Unsupported("browser runtime"))
    }

    pub fn save_workflow(&self, request: SaveWorkflowRequest) -> BrowserResult<StoredWorkflow> {
        self.repository.save_workflow(
            &request.id,
            request.version,
            request.workflow,
            request.selector_tree,
        )
    }
    pub fn workflow(&self, id: &str) -> BrowserResult<Option<StoredWorkflow>> {
        self.repository.workflow(id)
    }
    pub fn repository(&self) -> &Arc<BrowserRepository> {
        &self.repository
    }
    pub fn artifacts(&self) -> &Arc<BrowserArtifactStore> {
        &self.artifacts
    }
    pub fn enqueue_preview(
        &self,
        request: BrowserPreviewRequest,
        idempotency_key: &str,
    ) -> BrowserResult<BrowserJobDto> {
        self.jobs.enqueue(request, idempotency_key)
    }
    pub fn job(&self, id: &str) -> Option<BrowserJobDto> {
        self.jobs.get(id)
    }
    pub fn jobs(&self, limit: usize) -> Vec<BrowserJobDto> {
        self.jobs.list(limit)
    }
    pub fn cancel_job(&self, id: &str) -> BrowserResult<BrowserJobDto> {
        self.jobs.cancel(id)
    }
    pub async fn execute_one_job(&self, worker_id: &str) -> BrowserResult<Option<BrowserJobDto>> {
        let Some(lease) = self.jobs.claim(worker_id)? else {
            return Ok(None);
        };
        self.execute_lease(lease).await.map(Some)
    }
    async fn execute_lease(&self, lease: BrowserJobLease) -> BrowserResult<BrowserJobDto> {
        match self.preview(lease.request).await {
            Ok(result) => self.jobs.complete(
                &lease.job.id,
                &lease.worker_id,
                result.artifacts.iter().map(|a| a.id.clone()).collect(),
            ),
            Err(error) => {
                let failure = super::retry::BrowserRetryPolicy::classify(&error);
                let retryable = matches!(
                    failure,
                    super::retry::BrowserFailure::NavigationTransient
                        | super::retry::BrowserFailure::BrowserCrash
                        | super::retry::BrowserFailure::ProviderFailure
                        | super::retry::BrowserFailure::SessionExpired
                        | super::retry::BrowserFailure::TargetServerTransient
                );
                self.jobs
                    .fail(&lease.job.id, &lease.worker_id, &error, retryable)
            }
        }
    }
    #[cfg(feature = "browser")]
    pub fn session_ids(&self) -> Vec<String> {
        let mut ids = self
            .scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }
    #[cfg(not(feature = "browser"))]
    pub fn session_ids(&self) -> Vec<String> {
        Vec::new()
    }
    #[cfg(feature = "browser")]
    pub async fn close_session(&self, id: &str) -> BrowserResult<bool> {
        let scope = self
            .scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
            .ok_or(BrowserError::NotFound("browser session"))?;
        let result = self.runtime()?.close_session(&scope.runtime_key).await?;
        self.repository.remove_session(id)?;
        Ok(result)
    }

    #[cfg(feature = "browser")]
    fn resolve_scope(
        &self,
        request: &BrowserPreviewRequest,
        initial_host: &str,
    ) -> BrowserResult<(String, String, Vec<String>)> {
        if let Some(id) = &request.session_id {
            validate_session_id(id)?;
            let scope = self.scope_for(id)?;
            if !scope
                .allowed_hosts
                .iter()
                .any(|h| h.eq_ignore_ascii_case(initial_host))
            {
                return Err(BrowserError::Policy(
                    "preview URL is outside the existing session scope".into(),
                ));
            }
            return Ok((id.clone(), scope.runtime_key, scope.allowed_hosts));
        }
        let id = if request.request_id.len() <= 96 {
            request.request_id.clone()
        } else {
            return Err(BrowserError::InvalidInput("request id"));
        };
        validate_session_id(&id)?;
        let mut allowed = BTreeMap::<String, ()>::new();
        allowed.insert(initial_host.to_owned(), ());
        for h in self
            .config
            .authorized_hosts
            .iter()
            .chain(request.allowed_hosts.iter())
        {
            let h = h.trim().to_ascii_lowercase();
            if !h.is_empty() {
                allowed.insert(h, ());
            }
        }
        let hosts: Vec<String> = allowed.into_keys().collect();
        let key = format!("api-{id}");
        let entry = SessionScope {
            runtime_key: key.clone(),
            allowed_hosts: hosts.clone(),
            created_at_ms: super::page::now_ms(),
        };
        self.scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), entry);
        Ok((id, key, hosts))
    }
    #[cfg(feature = "browser")]
    fn scope_for(&self, id: &str) -> BrowserResult<SessionScope> {
        self.scopes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
            .ok_or(BrowserError::NotFound("browser session"))
    }
    #[cfg(feature = "browser")]
    async fn get_session(&self, id: &str) -> BrowserResult<Arc<BrowserSession>> {
        validate_session_id(id)?;
        let scope = self.scope_for(id)?;
        self.ensure_runtime().await?
            .session(&scope.runtime_key, None, None)
            .await
    }

    #[cfg(feature = "browser")]
    fn persist_session(
        &self,
        id: &str,
        key: &str,
        session: &BrowserSession,
        profile: Option<String>,
    ) -> BrowserResult<()> {
        let scope = self.scope_for(id)?;
        let _ = key;
        self.repository.save_session(BrowserSessionDto {
            id: id.into(),
            runtime_id: session.runtime_id().into(),
            state: "active".into(),
            created_at_ms: scope.created_at_ms,
            expires_at_ms: None,
            profile_id: profile,
            egress_id: None,
        })
    }
}

fn validate_session_id(id: &str) -> BrowserResult<()> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    {
        Err(BrowserError::InvalidInput("browser session id"))
    } else {
        Ok(())
    }
}
fn validate_preview(request: &BrowserPreviewRequest) -> BrowserResult<()> {
    if request.request_id.is_empty()
        || request.request_id.len() > 96
        || request.target_id.is_empty()
        || request.target_id.len() > 128
    {
        return Err(BrowserError::InvalidInput("preview request identifiers"));
    }
    let url = url::Url::parse(&request.target_url)
        .map_err(|_| BrowserError::InvalidInput("preview URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(BrowserError::Policy(
            "preview URL must be credential-free http(s)".into(),
        ));
    }
    if request.allowed_hosts.len() > 128 {
        return Err(BrowserError::ContentLimit {
            kind: "preview allowed hosts",
            limit: 128,
        });
    }
    request
        .workflow
        .validate()
        .map_err(|e| BrowserError::Config(e.to_string()))?;
    if let Some(tree) = &request.selector_tree {
        tree.validate()
            .map_err(|e| BrowserError::Config(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_rejects_credential_urls() {
        let r = BrowserPreviewRequest {
            request_id: "r".into(),
            target_id: "t".into(),
            target_url: "https://u:p@example.test".into(),
            session_id: None,
            allowed_hosts: vec![],
            workflow: Workflow::default(),
            selector_tree: None,
            capture_screenshot: false,
        };
        assert!(validate_preview(&r).is_err());
    }
    #[test]
    fn session_ids_are_path_safe() {
        assert!(validate_session_id("session-1").is_ok());
        assert!(validate_session_id("../../file").is_err());
    }
}
