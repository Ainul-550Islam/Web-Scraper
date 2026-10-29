//! Wire types for the managed customer API.
//!
//! Every response carries `"schema": 1` so a client can tell which contract
//! it is talking to, and request bodies are `deny_unknown_fields` so a typo
//! fails loudly at the edge instead of silently being ignored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::usage::ledger::RenderMode;

/// Schema version stamped on every managed API response.
pub const RESPONSE_SCHEMA: u32 = 1;

/// The renderer a customer asks for.
pub type Render = RenderMode;

/// What the caller wants back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    /// The response body verbatim.
    Html,
    /// Plain text, HTML tags stripped.
    Text,
    /// CommonMark.
    Markdown,
    /// The extracted JSON document.
    Json,
    /// A PNG screenshot; only valid with `render: "browser"`.
    Screenshot,
}

impl OutputFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "html" => Some(Self::Html),
            "text" => Some(Self::Text),
            "markdown" => Some(Self::Markdown),
            "json" => Some(Self::Json),
            "screenshot" => Some(Self::Screenshot),
            other => {
                let _ = other;
                None
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Text => "text",
            Self::Markdown => "markdown",
            Self::Json => "json",
            Self::Screenshot => "screenshot",
        }
    }
}

impl Default for OutputFormat {
    fn default() -> Self {
        Self::Html
    }
}

/// A request id is assigned per call so a response, a ledger entry and a
/// webhook delivery can all be correlated.
impl ScrapeRequest {
    pub fn request_id(&self) -> String {
        crate::ids::CorrelationId::new().to_string()
    }
}

/// A customer's request to retrieve one URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrapeRequest {
    /// Absolute `http(s)` URL. SSRF-checked before anything else.
    pub url: String,
    #[serde(default)]
    pub render: Render,
    /// Pin the proxy region (an id from the pool). Omit to let the pool
    /// choose.
    #[serde(default)]
    pub proxy_region: Option<String>,
    /// Optional stable browser-session key, so a repeat call resumes the
    /// same session instead of paying for a new one.
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub extract: ExtractSpec,
    #[serde(default)]
    pub format: OutputFormat,
    /// Take a screenshot as well as (or instead of) the body.
    #[serde(default)]
    pub screenshot: bool,
    /// Browser waits, applied before the body is captured.
    #[serde(default)]
    pub wait: WaitSpec,
    /// Signed callback for the finished request.
    #[serde(default)]
    pub webhook_url: Option<String>,
    /// Client-generated replay key; see the idempotency notes.
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

/// Field extraction, reusing the project's own rule types so a customer
/// writes the same thing an operator writes in a target config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractSpec {
    /// CSS (HTML mode) or JSONPath-lite (JSON mode) field rules.
    #[serde(default)]
    pub fields: BTreeMap<String, crate::config::FieldCfg>,
    /// A selector tree for structured extraction, in the shape the
    /// selector engine parses.
    #[serde(default)]
    pub selector: Option<serde_json::Value>,
    /// Upper bound on records the selector tree may emit.
    #[serde(default)]
    pub max_records: Option<usize>,
}

impl ExtractSpec {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty() && self.selector.is_none()
    }
}

/// Browser waits. All bounded; unset means "the renderer's default".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaitSpec {
    /// Milliseconds to settle for. Clamped to [`super::super::config`] bounds.
    #[serde(default)]
    pub ms: Option<u64>,
    /// CSS selector to wait for before capturing.
    #[serde(default)]
    pub until_selector: Option<String>,
    /// Extra settle time after the selector matches.
    #[serde(default)]
    pub settle_ms: Option<u64>,
}

/// A job, as `POST /v1/jobs` accepts it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateJobRequest {
    #[serde(flatten)]
    pub scrape: ScrapeRequest,
    /// How often the job repeats. 0 = once.
    #[serde(default)]
    pub schedule: JobSchedule,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobSchedule {
    #[serde(default)]
    pub interval_minutes: u32,
    /// How many times to run before the job stops. 0 = forever.
    #[serde(default)]
    pub repeat: u32,
}

/// Job lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Cancelled)
    }
}

/// One artifact handed back to the caller.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub id: String,
    pub kind: String,
    pub bytes: u64,
}

/// The last metering chain head, echoed so a caller can see the ledger
/// advanced (and an operator can audit which request advanced it).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChainHead {
    pub seq: u64,
    pub hash: String,
}

/// The canonical success envelope for `POST /v1/scrape`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrapeResponse {
    pub schema: u32,
    pub request_id: String,
    pub status: u16,
    pub url: String,
    pub final_url: String,
    pub content_type: String,
    pub bytes: u64,
    pub latency_ms: u64,
    pub render: String,
    pub format: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub extracted: Option<serde_json::Value>,
    #[serde(default)]
    pub records: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    pub usage_units: u32,
    pub cost: String,
    #[serde(default)]
    pub screenshot: Option<ArtifactRef>,
    pub chain: ChainHead,
}

/// The canonical response for `POST /v1/jobs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobCreateResponse {
    pub schema: u32,
    pub job_id: String,
    pub status: String,
    pub estimated_units: u32,
    pub idempotency_replayed: bool,
    pub chain: ChainHead,
}

/// The canonical response for `GET /v1/jobs/:id` and `POST /v1/jobs/:id/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobResponse {
    pub schema: u32,
    pub job_id: String,
    pub status: String,
    pub url: String,
    pub request_id: String,
    /// Populated once the job reaches a terminal state.
    #[serde(default)]
    pub scrape: Option<ScrapeResponse>,
    #[serde(default)]
    pub records: Vec<serde_json::Value>,
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    #[serde(default)]
    pub error: Option<ErrorBody>,
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub attempts: u8,
    pub cancelled: bool,
    pub chain: ChainHead,
}

/// The canonical error envelope. `status` is the HTTP status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
            retry_after_ms: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn with_retry_after(mut self, ms: u64) -> Self {
        self.retry_after_ms = Some(ms);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorResponse {
    pub schema: u32,
    pub error: ErrorBody,
}

impl ErrorResponse {
    pub fn new(body: ErrorBody) -> Self {
        Self {
            schema: RESPONSE_SCHEMA,
            error: body,
        }
    }
}

/// A webhook redelivery is signed with this header value.
pub const WEBHOOK_SIGNATURE_HEADER: &str = "X-Harvest-Signature";
/// The timestamp header, so a consumer can bound replay.
pub const WEBHOOK_TIMESTAMP_HEADER: &str = "X-Harvest-Timestamp";
