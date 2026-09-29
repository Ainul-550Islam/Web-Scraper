//! Authenticated control-plane command model.
//!
//! This module is the **typed front door** for remote operator actions. It
//! owns three things and delegates everything else:
//!
//! | Concern | Owner |
//! |---|---|
//! | Command shape, bounds, validation | this module |
//! | Idempotency of mutating submissions | [`IdempotencyIndex`] |
//! | Scheduling, execution, lifecycle, audit | [`crate::execution`] |
//! | Crawl execution | [`crate::engine`] (unchanged) |
//! | Export generation | [`crate::export`] (unchanged) |
//! | AI assistance | [`crate::ai`] (unchanged) |
//! | Artifact delivery | [`crate::download`] |
//!
//! Everything here is data plus a refusal. There is no second engine, no
//! second queue, and no second authentication system: a command is only ever
//! constructed by code that has already passed the admin API's bearer check
//! ([`crate::admin`]), and [`ControlCommand::validate`] re-checks the bounds
//! that the operator can influence.
//!
//! **Scope discipline.** A control-plane command may select, bound and run
//! work the engine already knows how to do. It may not invent new crawl
//! scope: a crawl command names *targets that already exist in the attached
//! configuration* and may only tighten their budgets (`max_pages`,
//! `max_depth`), never widen them. That keeps the audited config document as
//! the single declaration of what this deployment is allowed to collect.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::api::{epoch_secs, ApiError};
use crate::config::Config;
use crate::crawl::{MAX_DEPTH_CEILING, MAX_PAGES_CEILING};
use crate::export::ExportFormat;

/// Hard cap on a control-plane request body (64 KiB). Big enough for an AI
/// batch of fields, small enough that a hostile client cannot make the admin
/// task buffer.
pub const COMMAND_BODY_LIMIT: u64 = 65_536;

/// Ceilings a command may never exceed. They are deliberately coarse and
/// machine-independent: no deployment tuning is expressed here.
pub const MAX_RECORDS_CEILING: u64 = 5_000_000;
pub const MAX_EXPORT_BYTES_CEILING: u64 = 1_073_741_824; // 1 GiB
pub const MAX_AI_INPUT_CEILING: u64 = 4_194_304; // 4 MiB
pub const MAX_TIMEOUT_SECS: u64 = 3_600;
pub const MAX_AI_FIELDS: usize = 512;
pub const MAX_AI_FIELD_VALUE_BYTES: usize = 4_096;

/// Bounded concurrency/queue limits for the control plane. Rejecting cleanly
/// is the behaviour: an exhausted control plane says so instead of queueing
/// forever.
pub const MAX_CONCURRENT_JOBS: usize = 8;
pub const MAX_QUEUE_DEPTH: usize = 64;

/// Hard cap for one artifact download (256 MiB).
pub const MAX_DOWNLOAD_BYTES: u64 = 268_435_456;

/// Retained idempotency keys. Oldest-key eviction; memory is O(cap).
pub const IDEMPOTENCY_CAP: usize = 256;

// ------------------------------------------------------------------ identity

/// Process-unique command identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CommandId(pub String);

static CMD_SEQ: AtomicU64 = AtomicU64::new(1);

impl CommandId {
    pub fn next() -> Self {
        CommandId(format!("cmd-{}", CMD_SEQ.fetch_add(1, Ordering::Relaxed)))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CommandId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Control-plane job identity. Distinct from [`crate::job::JobId`] because
/// the granularities differ: a `crate::job::Job` is one URL fetch, a control
/// job is one whole crawl/export/AI round.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ControlJobId(pub String);

static JOB_SEQ: AtomicU64 = AtomicU64::new(1);

impl ControlJobId {
    pub fn next() -> Self {
        ControlJobId(format!("job-{}", JOB_SEQ.fetch_add(1, Ordering::Relaxed)))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Parse an id that came from a URL segment. Rejects anything that is not
    /// the exact shape we mint, so a path segment can never carry traversal
    /// payload into a lookup.
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("job-")?;
        if rest.is_empty() || rest.len() > 20 || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(ControlJobId(s.to_string()))
    }
}

impl std::fmt::Display for ControlJobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Artifact identity. Same mint-and-parse discipline as [`ControlJobId`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ArtifactId(pub String);

static ART_SEQ: AtomicU64 = AtomicU64::new(1);

impl ArtifactId {
    pub fn next() -> Self {
        ArtifactId(format!("art-{}", ART_SEQ.fetch_add(1, Ordering::Relaxed)))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn parse(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("art-")?;
        if rest.is_empty() || rest.len() > 20 || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(ArtifactId(s.to_string()))
    }
}

impl std::fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ----------------------------------------------------------------- operations

/// The five operations the control plane exposes. Every one maps onto
/// functionality that already exists; nothing here is a stub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Operation {
    /// Run the engine over configured targets (bounded).
    Crawl,
    /// Render a delivery from the run output with the existing exporters.
    Export,
    /// Optional AI assistance through the existing provider abstraction.
    AiAssist,
    /// Re-check a delivery's manifest/checksums.
    Verify,
    /// Inspect a registered artifact's metadata (never its bytes).
    ArtifactInspect,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Crawl => "crawl",
            Self::Export => "export",
            Self::AiAssist => "ai",
            Self::Verify => "verify",
            Self::ArtifactInspect => "artifact",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "crawl" => Self::Crawl,
            "export" => Self::Export,
            "ai" | "ai-assist" | "ai_assist" => Self::AiAssist,
            "verify" => Self::Verify,
            "artifact" | "artifact-inspect" => Self::ArtifactInspect,
            _ => return None,
        })
    }
    /// Mutating operations create jobs; the two inspection operations are
    /// answered synchronously from state the server already holds.
    pub fn creates_job(self) -> bool {
        matches!(self, Self::Crawl | Self::Export | Self::AiAssist)
    }
}

// --------------------------------------------------------------------- limits

/// Per-command resource bounds. Every field has a ceiling and every ceiling
/// is enforced in [`CommandLimits::validate`]; the executor additionally
/// clamps *down* to whatever the attached configuration allows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandLimits {
    pub max_pages: usize,
    pub max_depth: u32,
    pub max_records: u64,
    pub max_export_bytes: u64,
    pub max_ai_input_bytes: u64,
    pub timeout_secs: u64,
}

impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            max_pages: 100,
            max_depth: 1,
            max_records: 100_000,
            max_export_bytes: 268_435_456,
            max_ai_input_bytes: 1_048_576,
            timeout_secs: 900,
        }
    }
}

impl CommandLimits {
    /// Merge client-supplied overrides, ignoring absent fields.
    pub fn apply_overrides(&mut self, v: &Value) -> Result<(), ControlError> {
        let Some(obj) = v.as_object() else {
            return Err(ControlError::invalid("`limits` must be an object"));
        };
        for (k, val) in obj {
            match k.as_str() {
                "max_pages" => self.max_pages = usize_of(val, "limits.max_pages")?,
                "max_depth" => self.max_depth = u32_of(val, "limits.max_depth")?,
                "max_records" => self.max_records = u64_of(val, "limits.max_records")?,
                "max_export_bytes" => {
                    self.max_export_bytes = u64_of(val, "limits.max_export_bytes")?
                }
                "max_ai_input_bytes" => {
                    self.max_ai_input_bytes = u64_of(val, "limits.max_ai_input_bytes")?
                }
                "timeout_secs" => self.timeout_secs = u64_of(val, "limits.timeout_secs")?,
                other => {
                    return Err(ControlError::invalid(format!(
                        "unknown limit `{other}` (expected max_pages, max_depth, max_records, max_export_bytes, max_ai_input_bytes, timeout_secs)"
                    )))
                }
            }
        }
        Ok(())
    }

    /// Refuse anything above a ceiling. Zero is refused for every bound that
    /// means "how much work": an explicit zero is almost always a bug, and a
    /// job that does nothing is still a job somebody has to reason about.
    pub fn validate(&self) -> Result<(), ControlError> {
        if self.max_pages == 0 {
            return Err(ControlError::invalid("limits.max_pages must be at least 1"));
        }
        if self.max_pages > MAX_PAGES_CEILING {
            return Err(ControlError::invalid(format!(
                "limits.max_pages {} exceeds the ceiling {}",
                self.max_pages, MAX_PAGES_CEILING
            )));
        }
        if self.max_depth > MAX_DEPTH_CEILING {
            return Err(ControlError::invalid(format!(
                "limits.max_depth {} exceeds the ceiling {}",
                self.max_depth, MAX_DEPTH_CEILING
            )));
        }
        if self.max_records == 0 {
            return Err(ControlError::invalid(
                "limits.max_records must be at least 1",
            ));
        }
        if self.max_records > MAX_RECORDS_CEILING {
            return Err(ControlError::invalid(format!(
                "limits.max_records {} exceeds the ceiling {}",
                self.max_records, MAX_RECORDS_CEILING
            )));
        }
        if self.max_export_bytes == 0 {
            return Err(ControlError::invalid(
                "limits.max_export_bytes must be at least 1",
            ));
        }
        if self.max_export_bytes > MAX_EXPORT_BYTES_CEILING {
            return Err(ControlError::invalid(format!(
                "limits.max_export_bytes {} exceeds the ceiling {}",
                self.max_export_bytes, MAX_EXPORT_BYTES_CEILING
            )));
        }
        if self.max_ai_input_bytes == 0 {
            return Err(ControlError::invalid(
                "limits.max_ai_input_bytes must be at least 1",
            ));
        }
        if self.max_ai_input_bytes > MAX_AI_INPUT_CEILING {
            return Err(ControlError::invalid(format!(
                "limits.max_ai_input_bytes {} exceeds the ceiling {}",
                self.max_ai_input_bytes, MAX_AI_INPUT_CEILING
            )));
        }
        if self.timeout_secs == 0 {
            return Err(ControlError::invalid(
                "limits.timeout_secs must be at least 1",
            ));
        }
        if self.timeout_secs > MAX_TIMEOUT_SECS {
            return Err(ControlError::invalid(format!(
                "limits.timeout_secs {} exceeds the ceiling {}",
                self.timeout_secs, MAX_TIMEOUT_SECS
            )));
        }
        Ok(())
    }

    /// Tighten these limits so they never exceed the attached configuration's
    /// own bounds. Called by the executor, which is the only place a `Config`
    /// is guaranteed to exist.
    pub fn clamp_to_config(&mut self, cfg: &Config) {
        if cfg.max_pages > 0 {
            self.max_pages = self.max_pages.min(cfg.max_pages);
        }
        for t in &cfg.targets {
            // depth is per-target; the command bound is a ceiling over all of
            // them, so take the smallest configured depth as the tighter one.
            let d = t.max_depth as u32;
            self.max_depth = self.max_depth.min(d.min(MAX_DEPTH_CEILING));
        }
        if cfg.max_records_per_run > 0 {
            self.max_records = self.max_records.min(cfg.max_records_per_run);
        }
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

// ------------------------------------------------------------------- requests

/// Crawl: select configured targets and (optionally) tighten their budgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrawlRequest {
    /// Target ids declared in the attached configuration. Empty means "all
    /// configured targets" for a single-target config, and is refused when
    /// the config declares more than one (naming the scope is the point).
    pub targets: Vec<String>,
}

/// Export: render a delivery from the run output with the existing exporters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportRequest {
    /// Format ids; unknown ids are refused with the supported list.
    pub formats: Vec<String>,
    /// Optional dataset id used for file naming (`dataset-<id>.<ext>`).
    pub dataset: Option<String>,
    /// Include the ZIP bundle of every selected format + manifest + checksums.
    pub bundle: bool,
    /// Run `harvest verify` semantics on the finished delivery and refuse to
    /// mark its artifacts downloadable unless it passes.
    pub verify: bool,
    /// Stop after this many records (bounded by `limits.max_records`).
    pub max_records: Option<u64>,
}

/// Where an AI job's input comes from. Both paths are bounded and both are
/// redacted by the existing [`crate::ai::redact`] pass before a provider sees
/// anything.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AiSource {
    /// Read records from the run output the attached config declares.
    RunOutput {
        record_limit: usize,
        field_limit: usize,
    },
    /// Fields supplied by the operator (bounded by count and value size).
    Inline { fields: Vec<AiFieldInput> },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiFieldInput {
    pub name: String,
    pub value: String,
    /// The deterministic value already extracted, when there is one. AI
    /// suggestions never replace one of these without explicit policy.
    #[serde(default)]
    pub deterministic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiRequest {
    /// One of `extract`, `classify`, `normalize`, `schema-suggest`, `label`.
    pub task: String,
    pub source: AiSource,
    /// Suggestions below this confidence are reported as `review`, never
    /// applied. Range [0,1].
    pub confidence: f64,
    /// Provider id. Only locally-implemented providers resolve; anything
    /// external is refused unless the deployment opted in *and* the provider
    /// is registered — which this build never does.
    pub provider: String,
    /// Consent to send data to an external provider. Recorded and audited;
    /// it never causes an external provider to appear.
    pub external_allowed: bool,
}

/// Verify: re-check a delivery that a previous export job produced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerifyRequest {
    /// Control job id of a completed export job.
    pub job: Option<String>,
    /// Or a registered artifact id belonging to a delivery.
    pub artifact: Option<String>,
}

/// Artifact inspection: metadata only, never bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactInspectRequest {
    pub artifact: String,
}

/// The typed payload of a command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CommandSpec {
    Crawl(CrawlRequest),
    Export(ExportRequest),
    AiAssist(AiRequest),
    Verify(VerifyRequest),
    ArtifactInspect(ArtifactInspectRequest),
}

impl CommandSpec {
    pub fn operation(&self) -> Operation {
        match self {
            Self::Crawl(_) => Operation::Crawl,
            Self::Export(_) => Operation::Export,
            Self::AiAssist(_) => Operation::AiAssist,
            Self::Verify(_) => Operation::Verify,
            Self::ArtifactInspect(_) => Operation::ArtifactInspect,
        }
    }

    /// Parse a request body. Unknown object members are ignored (clients may
    /// send more than we read), but every member we *do* read is type-checked
    /// and range-checked: a malformed command is refused, never coerced.
    pub fn from_json(v: &Value) -> Result<Self, ControlError> {
        let Some(obj) = v.as_object() else {
            return Err(ControlError::invalid("body must be a JSON object"));
        };
        let op_raw = obj.get("op").and_then(Value::as_str).ok_or_else(|| {
            ControlError::invalid("missing `op` (crawl | export | ai | verify | artifact)")
        })?;
        let Some(op) = Operation::parse(op_raw) else {
            return Err(ControlError::invalid(format!(
                "unsupported op `{op_raw}` (expected crawl, export, ai, verify or artifact)"
            )));
        };
        let get = |k: &str| obj.get(k);
        match op {
            Operation::Crawl => {
                let targets = match get("targets") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(Value::Array(a)) => {
                        let mut out = Vec::new();
                        for item in a {
                            let s = item.as_str().ok_or_else(|| {
                                ControlError::invalid("targets[] must be strings")
                            })?;
                            if !is_safe_id(s) {
                                return Err(ControlError::invalid(format!(
                                    "target id `{s}` contains unsupported characters"
                                )));
                            }
                            out.push(s.to_string());
                        }
                        out
                    }
                    Some(_) => {
                        return Err(ControlError::invalid(
                            "`targets` must be an array of strings",
                        ))
                    }
                };
                Ok(Self::Crawl(CrawlRequest { targets }))
            }
            Operation::Export => {
                let formats: Vec<String> = match get("formats") {
                    Some(Value::Array(a)) => {
                        let mut out = Vec::new();
                        for item in a {
                            let s = item.as_str().ok_or_else(|| {
                                ControlError::invalid("formats[] must be strings")
                            })?;
                            out.push(s.to_string());
                        }
                        out
                    }
                    Some(Value::Null) | None => {
                        return Err(ControlError::invalid(
                            "`formats` is required (csv, jsonl, xlsx, pdf, parquet, xml, zip)",
                        ))
                    }
                    Some(_) => {
                        return Err(ControlError::invalid(
                            "`formats` must be an array of strings",
                        ))
                    }
                };
                let dataset = optional_string(get("dataset"), "dataset")?;
                if let Some(d) = &dataset {
                    if d.len() > 64 {
                        return Err(ControlError::invalid(
                            "`dataset` must be at most 64 characters",
                        ));
                    }
                }
                let max_records = match get("max_records") {
                    None | Some(Value::Null) => None,
                    Some(v) => Some(u64_of(v, "max_records")?),
                };
                Ok(Self::Export(ExportRequest {
                    formats,
                    dataset,
                    bundle: bool_of(get("bundle"), "bundle")?,
                    verify: bool_of(get("verify"), "verify")?,
                    max_records,
                }))
            }
            Operation::AiAssist => {
                let task = obj
                    .get("task")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ControlError::invalid("missing `task`"))?
                    .to_string();
                let confidence = match get("confidence") {
                    None | Some(Value::Null) => crate::ai::schema::Thresholds::default().accept,
                    Some(v) => {
                        let c = v.as_f64().ok_or_else(|| {
                            ControlError::invalid("`confidence` must be a number")
                        })?;
                        if !(0.0..=1.0).contains(&c) {
                            return Err(ControlError::invalid(
                                "`confidence` must be within [0, 1]",
                            ));
                        }
                        c
                    }
                };
                let provider = obj
                    .get("provider")
                    .and_then(Value::as_str)
                    .unwrap_or(LOCAL_PROVIDER_ID)
                    .to_string();
                let external_allowed = bool_of(get("external_allowed"), "external_allowed")?;
                let source = match get("source") {
                    None | Some(Value::Null) => AiSource::RunOutput {
                        record_limit: 100,
                        field_limit: 32,
                    },
                    Some(Value::Object(o)) => {
                        let kind = o
                            .get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("run_output");
                        match kind {
                            "run_output" | "run-output" => AiSource::RunOutput {
                                record_limit: match o.get("record_limit") {
                                    None => 100,
                                    Some(v) => {
                                        usize_of(v, "source.record_limit")?.min(MAX_AI_FIELDS)
                                    }
                                },
                                field_limit: match o.get("field_limit") {
                                    None => 32,
                                    Some(v) => {
                                        usize_of(v, "source.field_limit")?.min(MAX_AI_FIELDS)
                                    }
                                },
                            },
                            "inline" => {
                                let Some(Value::Array(a)) = o.get("fields") else {
                                    return Err(ControlError::invalid(
                                        "source.fields must be an array when kind is `inline`",
                                    ));
                                };
                                if a.is_empty() {
                                    return Err(ControlError::invalid(
                                        "source.fields must not be empty",
                                    ));
                                }
                                if a.len() > MAX_AI_FIELDS {
                                    return Err(ControlError::invalid(format!(
                                        "source.fields exceeds the {MAX_AI_FIELDS} field limit"
                                    )));
                                }
                                let mut fields = Vec::new();
                                for item in a {
                                    let Some(fo) = item.as_object() else {
                                        return Err(ControlError::invalid(
                                            "source.fields[] must be objects",
                                        ));
                                    };
                                    let name = fo
                                        .get("name")
                                        .and_then(Value::as_str)
                                        .ok_or_else(|| {
                                            ControlError::invalid(
                                                "source.fields[].name is required",
                                            )
                                        })?
                                        .to_string();
                                    if name.len() > 64 {
                                        return Err(ControlError::invalid(
                                            "source.fields[].name must be at most 64 characters",
                                        ));
                                    }
                                    let value = match fo.get("value") {
                                        Some(Value::String(s)) => s.clone(),
                                        Some(other) => other.to_string(),
                                        None => String::new(),
                                    };
                                    if value.len() > MAX_AI_FIELD_VALUE_BYTES {
                                        return Err(ControlError::invalid(format!(
                                            "source.fields[].value exceeds {MAX_AI_FIELD_VALUE_BYTES} bytes"
                                        )));
                                    }
                                    let deterministic =
                                        optional_string(fo.get("deterministic"), "deterministic")?;
                                    fields.push(AiFieldInput {
                                        name,
                                        value,
                                        deterministic,
                                    });
                                }
                                AiSource::Inline { fields }
                            }
                            other => {
                                return Err(ControlError::invalid(format!(
                                "unsupported source kind `{other}` (expected run_output or inline)"
                            )))
                            }
                        }
                    }
                    Some(_) => return Err(ControlError::invalid("`source` must be an object")),
                };
                Ok(Self::AiAssist(AiRequest {
                    task,
                    source,
                    confidence,
                    provider,
                    external_allowed,
                }))
            }
            Operation::Verify => {
                let job = optional_string(get("job"), "job")?;
                let artifact = optional_string(get("artifact"), "artifact")?;
                if job.is_none() && artifact.is_none() {
                    return Err(ControlError::invalid(
                        "verify needs `job` (a completed export job) or `artifact`",
                    ));
                }
                Ok(Self::Verify(VerifyRequest { job, artifact }))
            }
            Operation::ArtifactInspect => {
                let artifact = obj
                    .get("artifact")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ControlError::invalid("missing `artifact`"))?
                    .to_string();
                Ok(Self::ArtifactInspect(ArtifactInspectRequest { artifact }))
            }
        }
    }
}

/// The only provider id this build can resolve locally.
pub const LOCAL_PROVIDER_ID: &str = "deterministic-local";
/// The provider id used when AI is switched off.
pub const DISABLED_PROVIDER_ID: &str = "disabled";

// -------------------------------------------------------------------- command

/// A validated operator request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlCommand {
    pub id: CommandId,
    /// Correlation id of the HTTP request that created it.
    pub corr: String,
    /// Coarse actor label ("admin-api"). Never a token.
    pub actor: String,
    pub spec: CommandSpec,
    pub limits: CommandLimits,
    /// Present when the client sent an `Idempotency-Key`.
    pub idempotency_key: Option<String>,
    /// sha256 of the raw request body — what makes a key reuse a conflict.
    pub body_hash: String,
    pub created_ts: u64,
}

impl ControlCommand {
    /// Build a command from an already-authenticated request. `body` is the
    /// raw bytes (hashed for idempotency, never stored).
    pub fn from_request(
        actor: &str,
        corr: &str,
        body: &[u8],
        idempotency_key: Option<String>,
    ) -> Result<Self, ControlError> {
        if body.is_empty() {
            return Err(ControlError::invalid(
                "a control command needs a JSON body with an `op`",
            ));
        }
        if body.len() as u64 > COMMAND_BODY_LIMIT {
            return Err(ControlError::too_large(COMMAND_BODY_LIMIT));
        }
        let v: Value = serde_json::from_slice(body)
            .map_err(|e| ControlError::invalid(format!("body is not valid JSON: {e}")))?;
        let spec = CommandSpec::from_json(&v)?;
        let mut limits = CommandLimits::default();
        if let Some(l) = v.get("limits") {
            if !l.is_null() {
                limits.apply_overrides(l)?;
            }
        }
        limits.validate()?;
        Ok(Self {
            id: CommandId::next(),
            corr: corr.to_string(),
            actor: actor.to_string(),
            spec,
            limits,
            idempotency_key,
            body_hash: sha256_hex(body),
            created_ts: epoch_secs(),
        })
    }

    pub fn operation(&self) -> Operation {
        self.spec.operation()
    }

    /// Full validation, including the checks that need the attached
    /// configuration. Called before a job is created; a failure here is a
    /// 400/503 to the caller, never a job.
    pub fn validate(&self, cfg: Option<&Config>) -> Result<(), ControlError> {
        self.limits.validate()?;
        match &self.spec {
            CommandSpec::Crawl(c) => {
                let Some(cfg) = cfg else {
                    return Err(ControlError::unavailable(
                        "no configuration is attached to this server; crawl commands cannot be scoped",
                    ));
                };
                if cfg.targets.is_empty() {
                    return Err(ControlError::unavailable(
                        "the attached configuration declares no targets",
                    ));
                }
                if c.targets.is_empty() && cfg.targets.len() > 1 {
                    return Err(ControlError::invalid(format!(
                        "the attached configuration declares {} targets; name the ones to crawl",
                        cfg.targets.len()
                    )));
                }
                for id in &c.targets {
                    if !cfg.targets.iter().any(|t| &t.id == id) {
                        return Err(ControlError::invalid(format!(
                            "unknown target `{id}`; the attached configuration declares: {}",
                            cfg.targets
                                .iter()
                                .map(|t| t.id.clone())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }
                }
                Ok(())
            }
            CommandSpec::Export(e) => {
                if e.formats.is_empty() {
                    return Err(ControlError::invalid(
                        "`formats` must name at least one format",
                    ));
                }
                let mut parsed: Vec<ExportFormat> = Vec::new();
                for f in &e.formats {
                    let Some(fmt) = ExportFormat::parse(f) else {
                        return Err(ControlError::invalid(format!(
                            "unsupported export format `{f}` (supported: {})",
                            ExportFormat::all()
                                .iter()
                                .map(|f| f.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    };
                    if !parsed.contains(&fmt) {
                        parsed.push(fmt);
                    }
                }
                if e.bundle && parsed.len() == 1 && parsed[0] == ExportFormat::Zip {
                    return Err(ControlError::invalid(
                        "`bundle` needs at least one payload format alongside zip",
                    ));
                }
                if let Some(n) = e.max_records {
                    if n == 0 {
                        return Err(ControlError::invalid("`max_records` must be at least 1"));
                    }
                    if n > MAX_RECORDS_CEILING {
                        return Err(ControlError::invalid(format!(
                            "`max_records` {n} exceeds the ceiling {MAX_RECORDS_CEILING}"
                        )));
                    }
                }
                let Some(cfg) = cfg else {
                    return Err(ControlError::unavailable(
                        "no configuration is attached to this server; export has no declared source",
                    ));
                };
                crate::export_job::source_files(cfg).map(|_| ())
            }
            CommandSpec::AiAssist(a) => {
                if crate::ai::AiTask::parse_task(&a.task).is_none() {
                    return Err(ControlError::invalid(format!(
                        "unsupported ai task `{}` (expected extract, classify, normalize, schema-suggest or label)",
                        a.task
                    )));
                }
                if !(0.0..=1.0).contains(&a.confidence) {
                    return Err(ControlError::invalid("`confidence` must be within [0, 1]"));
                }
                match a.provider.as_str() {
                    DISABLED_PROVIDER_ID | LOCAL_PROVIDER_ID => {}
                    other => {
                        return Err(ControlError::invalid(format!(
                            "unknown ai provider `{other}` (available: {LOCAL_PROVIDER_ID}, {DISABLED_PROVIDER_ID})"
                        )))
                    }
                }
                // Privacy default: external processing is off unless the
                // operator explicitly consents. Consent alone still cannot
                // summon an external provider — this build registers none.
                if a.external_allowed && a.provider == DISABLED_PROVIDER_ID {
                    return Err(ControlError::invalid(
                        "`external_allowed` has no effect for the disabled provider",
                    ));
                }
                match &a.source {
                    AiSource::RunOutput { .. } => {
                        let Some(cfg) = cfg else {
                            return Err(ControlError::unavailable(
                                "no configuration is attached; an ai job over the run output has no declared source",
                            ));
                        };
                        // Same source discipline as an export: only the file
                        // the configuration declares, and it must exist.
                        crate::export_job::source_files(cfg).map(|_| ())
                    }
                    AiSource::Inline { fields } => {
                        if fields.is_empty() {
                            return Err(ControlError::invalid("source.fields must not be empty"));
                        }
                        Ok(())
                    }
                }
            }
            CommandSpec::Verify(v) => {
                let has_job = v.job.is_some();
                let has_art = v.artifact.is_some();
                if has_job == has_art {
                    return Err(ControlError::invalid(
                        "verify takes exactly one of `job` or `artifact`",
                    ));
                }
                if let Some(id) = &v.job {
                    if ControlJobId::parse(id).is_none() {
                        return Err(ControlError::invalid("`job` is not a control job id"));
                    }
                }
                if let Some(id) = &v.artifact {
                    if ArtifactId::parse(id).is_none() {
                        return Err(ControlError::invalid("`artifact` is not an artifact id"));
                    }
                }
                Ok(())
            }
            CommandSpec::ArtifactInspect(a) => {
                if ArtifactId::parse(&a.artifact).is_none() {
                    return Err(ControlError::invalid("`artifact` is not an artifact id"));
                }
                Ok(())
            }
        }
    }

    /// Safe audit metadata for this command — an operation, a coarse
    /// resource label and the ids. No tokens, no paths, no payloads.
    pub fn audit_target(&self) -> Option<String> {
        match &self.spec {
            CommandSpec::Crawl(c) if !c.targets.is_empty() => Some(c.targets.join(",")),
            CommandSpec::Export(e) => Some(e.formats.join(",")),
            CommandSpec::AiAssist(a) => Some(a.task.clone()),
            CommandSpec::Verify(v) => v.job.clone().or_else(|| v.artifact.clone()),
            CommandSpec::ArtifactInspect(a) => Some(a.artifact.clone()),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------- errors

/// Why a control-plane request was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlErrorKind {
    /// The caller is not authenticated. Reused verbatim from the admin API so
    /// the reply is content-constant with every other route's 401.
    Unauthorized,
    /// Authenticated but not permitted for this resource.
    Forbidden,
    /// Malformed, unsupported, or out-of-bounds command.
    Invalid,
    /// Named resource does not exist (or is not visible to this actor).
    NotFound,
    /// Body or requested work exceeds a published limit.
    TooLarge,
    /// The request conflicts with current state (terminal job, key reuse).
    Conflict,
    /// The control plane cannot do this right now (no config, at capacity).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlError {
    pub kind: ControlErrorKind,
    /// Operator-facing text: names parameters and bounds, never secrets,
    /// never filesystem paths.
    pub message: String,
}

impl ControlError {
    pub fn unauthorized() -> Self {
        Self {
            kind: ControlErrorKind::Unauthorized,
            message: crate::admin::UNAUTHORIZED_MESSAGE.to_string(),
        }
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            kind: ControlErrorKind::Forbidden,
            message: msg.into(),
        }
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self {
            kind: ControlErrorKind::Invalid,
            message: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            kind: ControlErrorKind::NotFound,
            message: msg.into(),
        }
    }
    pub fn too_large(limit: u64) -> Self {
        Self {
            kind: ControlErrorKind::TooLarge,
            message: format!("request exceeds the {limit} byte limit"),
        }
    }
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            kind: ControlErrorKind::Conflict,
            message: msg.into(),
        }
    }
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            kind: ControlErrorKind::Unavailable,
            message: msg.into(),
        }
    }

    /// Map onto the one error shape every admin route already returns.
    pub fn to_api(&self) -> ApiError {
        match self.kind {
            ControlErrorKind::Unauthorized => ApiError::unauthorized(&self.message),
            ControlErrorKind::Forbidden => ApiError::forbidden(&self.message),
            ControlErrorKind::Invalid => ApiError::invalid(&self.message),
            ControlErrorKind::NotFound => ApiError::not_found(&self.message),
            ControlErrorKind::TooLarge => ApiError {
                status: 413,
                code: "payload_too_large",
                message: self.message.clone(),
            },
            ControlErrorKind::Conflict => ApiError::conflict(&self.message),
            ControlErrorKind::Unavailable => ApiError::unavailable(&self.message),
        }
    }

    /// HTTP status, for callers that only need the number.
    pub fn status(&self) -> u16 {
        match self.kind {
            ControlErrorKind::Unauthorized => 401,
            ControlErrorKind::Forbidden => 403,
            ControlErrorKind::Invalid => 400,
            ControlErrorKind::NotFound => 404,
            ControlErrorKind::TooLarge => 413,
            ControlErrorKind::Conflict => 409,
            ControlErrorKind::Unavailable => 503,
        }
    }
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ControlError {}

// --------------------------------------------------------------- idempotency

/// Outcome of an idempotency check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Idempotency {
    /// Key is new (or absent): the caller may proceed and must call
    /// [`IdempotencyIndex::bind`] with the job it creates.
    Fresh,
    /// The same key with the same body was seen before: replay this job
    /// instead of starting a second one.
    Replay(String),
    /// The key was reused with a different body: refuse.
    Conflict,
}

/// What a key is bound to once a job exists.
#[derive(Debug, Clone)]
pub struct IdempotencyRecord {
    pub key: String,
    pub actor: String,
    pub op: Operation,
    pub body_hash: String,
    pub job: Option<String>,
    pub created_ts: u64,
    pub seq: u64,
}

/// Bounded, oldest-evicted index of idempotency keys.
///
/// Semantics (explicit, because "idempotent" means different things to
/// different clients): a key is scoped to `(actor, op, sha256(body))`. Replay
/// returns the *original* job; reuse with a different body is a 409; keys are
/// retained for at most [`IDEMPOTENCY_CAP`] entries and are process-local, so
/// a server restart legitimately forgets them.
#[derive(Debug, Default)]
pub struct IdempotencyIndex {
    inner: Mutex<BTreeMap<String, IdempotencyRecord>>,
    seq: AtomicU64,
}

impl IdempotencyIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn check(&self, actor: &str, op: Operation, key: &str, body_hash: &str) -> Idempotency {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        match g.get(key) {
            None => Idempotency::Fresh,
            Some(r) => {
                if r.actor != actor || r.op != op || r.body_hash != body_hash {
                    Idempotency::Conflict
                } else {
                    match &r.job {
                        Some(j) => Idempotency::Replay(j.clone()),
                        None => Idempotency::Fresh,
                    }
                }
            }
        }
    }

    /// Bind a key to a job id. Evicts the oldest record when at capacity.
    pub fn bind(&self, actor: &str, op: Operation, key: &str, body_hash: &str, job: &str) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() >= IDEMPOTENCY_CAP && !g.contains_key(key) {
            let mut oldest: Option<(String, u64)> = None;
            for (k, r) in g.iter() {
                if oldest.as_ref().map(|(_, s)| r.seq < *s).unwrap_or(true) {
                    oldest = Some((k.clone(), r.seq));
                }
            }
            if let Some((k, _)) = oldest {
                g.remove(&k);
            }
        }
        g.insert(
            key.to_string(),
            IdempotencyRecord {
                key: key.to_string(),
                actor: actor.to_string(),
                op,
                body_hash: body_hash.to_string(),
                job: Some(job.to_string()),
                created_ts: epoch_secs(),
                seq,
            },
        );
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ------------------------------------------------------------------- helpers

/// Conservative id charset for ids that appear in audit rows, artifact
/// names and lookups. Deliberately narrow: no separators, no dots.
pub fn is_safe_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Idempotency keys are client-supplied: validate before they reach a map.
pub fn validate_idempotency_key(raw: Option<&str>) -> Result<Option<String>, ControlError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let key = raw.trim();
    if key.is_empty() || key.len() > 64 {
        return Err(ControlError::invalid(
            "`Idempotency-Key` must be 1–64 characters",
        ));
    }
    if !key
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ControlError::invalid(
            "`Idempotency-Key` may only contain letters, digits, `-` and `_`",
        ));
    }
    Ok(Some(key.to_string()))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn usize_of(v: &Value, field: &str) -> Result<usize, ControlError> {
    let n = v.as_u64().ok_or_else(|| {
        ControlError::invalid(format!("`{field}` must be a non-negative integer"))
    })?;
    usize::try_from(n)
        .map_err(|_| ControlError::invalid(format!("`{field}` is too large for this platform")))
}

fn u32_of(v: &Value, field: &str) -> Result<u32, ControlError> {
    let n = v.as_u64().ok_or_else(|| {
        ControlError::invalid(format!("`{field}` must be a non-negative integer"))
    })?;
    u32::try_from(n)
        .map_err(|_| ControlError::invalid(format!("`{field}` must be at most {}", u32::MAX)))
}

fn u64_of(v: &Value, field: &str) -> Result<u64, ControlError> {
    v.as_u64()
        .ok_or_else(|| ControlError::invalid(format!("`{field}` must be a non-negative integer")))
}

fn bool_of(v: Option<&Value>, field: &str) -> Result<bool, ControlError> {
    match v {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(ControlError::invalid(format!(
            "`{field}` must be a boolean"
        ))),
    }
}

fn optional_string(v: Option<&Value>, field: &str) -> Result<Option<String>, ControlError> {
    match v {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(ControlError::invalid(format!("`{field}` must be a string"))),
    }
}

#[cfg(test)]
mod tests;
