//! Stable browser-control API DTOs. Secret values are excluded from models.
use super::challenge::ChallengeObservation;
use super::page::{PageArtifactRef, PageObservation};
use super::workflow::Workflow;
use crate::selector::ast::SelectorTree;
use crate::selector::record::Record;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserPreviewRequest {
    pub request_id: String,
    pub target_id: String,
    pub target_url: String,
    pub session_id: Option<String>,
    pub allowed_hosts: Vec<String>,
    pub workflow: Workflow,
    pub selector_tree: Option<SelectorTree>,
    pub capture_screenshot: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserPreviewResponse {
    pub request_id: String,
    pub page: PageObservation,
    pub challenge: ChallengeObservation,
    pub records: Vec<Record>,
    pub artifacts: Vec<PageArtifactRef>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserNavigateRequest {
    pub session_id: String,
    pub url: String,
    pub allowed_hosts: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserInspectRequest {
    pub session_id: String,
    pub capture_dom: bool,
    pub capture_screenshot: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SelectorTestRequest {
    pub session_id: String,
    pub selector_tree: SelectorTree,
    pub max_records: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserActionRequest {
    pub session_id: String,
    pub action: Workflow,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveWorkflowRequest {
    pub id: String,
    pub workflow: Workflow,
    pub selector_tree: Option<SelectorTree>,
    pub version: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserSessionDto {
    pub id: String,
    pub runtime_id: String,
    pub state: String,
    pub created_at_ms: u64,
    pub expires_at_ms: Option<u64>,
    pub profile_id: Option<String>,
    pub egress_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserDiagnostics {
    pub enabled: bool,
    pub feature_compiled: bool,
    pub runtime_ready: bool,
    pub active_sessions: usize,
    pub max_sessions: usize,
    pub engine: String,
    pub last_error: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserJobDto {
    pub id: String,
    pub target_id: String,
    pub state: String,
    pub idempotency_key: String,
    pub body_digest: String,
    pub attempts: u32,
    pub queued_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub error: Option<String>,
    pub artifact_ids: Vec<String>,
}
