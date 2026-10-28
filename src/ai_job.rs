//! AI-assistance jobs: the existing AI client, wrapped in an authenticated,
//! budgeted lifecycle.
//!
//! No provider logic lives here. The provider interface, the request/response
//! schema, redaction, confidence thresholds, validation and the budget/timeout
//! rules are all [`crate::ai`]; this module only decides *which registered
//! provider* a job may use, what it is allowed to see, and how the outcome is
//! reported.
//!
//! Policy enforced here (and nowhere else, so there is one place to audit):
//!
//! * **Off by default.** A job that names `disabled`, or that runs on a build
//!   whose provider table has no entry, yields [`crate::ai::schema::AiState::Disabled`].
//! * **External processing requires explicit consent.** `external_allowed`
//!   is recorded and audited; it cannot summon a provider that is not
//!   registered, and this build registers exactly one provider
//!   (`deterministic-local`), which is local and external-free.
//! * **No credentials exist, so none can leak.** Providers are constructed
//!   from the request; the response carries provider *name/model* only.
//! * **Suggestions never overwrite validated data.** The job applies the
//!   outcome to a scratch record, never to the run output, and reports the
//!   accept/review/reject split the same way the pipeline does.
//! * **Deterministic data is preserved.** A failing, timing-out or malformed
//!   provider round leaves the source records untouched and says which of
//!   those happened.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::schema::{AiState, Thresholds};
use crate::ai::{
    AiCfg, AiClient, AiField, AiProvider, AiRequest, AiTask, DeterministicProvider,
    DisabledProvider,
};
use crate::config::Config;
use crate::control::{AiRequest as AiCommand, AiSource, CommandLimits, ControlError};

/// A validated AI request, resolved against the command's limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiJobSpec {
    pub task: AiTask,
    pub source: AiSource,
    pub confidence: f64,
    pub provider: String,
    pub external_allowed: bool,
    /// Budget for this one job (not global): one request, bounded input.
    pub request_budget: u32,
    pub max_input_bytes: u64,
    pub timeout_ms: u64,
    pub max_retries: u8,
}

impl AiJobSpec {
    pub fn from_request(r: &AiCommand, limits: &CommandLimits) -> Result<Self, ControlError> {
        let task = AiTask::parse_task(&r.task)
            .ok_or_else(|| ControlError::invalid(format!("unsupported ai task `{}`", r.task)))?;
        match r.provider.as_str() {
            crate::control::LOCAL_PROVIDER_ID | crate::control::DISABLED_PROVIDER_ID => {}
            other => {
                return Err(ControlError::invalid(format!(
                    "unknown ai provider `{other}`"
                )))
            }
        }
        Ok(Self {
            task,
            source: r.source.clone(),
            confidence: r.confidence,
            provider: r.provider.clone(),
            external_allowed: r.external_allowed,
            request_budget: 1,
            max_input_bytes: limits.max_ai_input_bytes,
            timeout_ms: limits.timeout_secs.saturating_mul(1000).min(120_000),
            // AI-specific retry budget, from the same defaults the pipeline
            // uses; a control job does not get more attempts than a run.
            max_retries: 1,
        })
    }

    /// The provider table. Adding an entry here is the *only* way a new
    /// provider becomes reachable, which is why it is a four-line function
    /// instead of a registry a request can fill.
    fn provider(&self) -> Box<dyn AiProvider> {
        match self.provider.as_str() {
            crate::control::LOCAL_PROVIDER_ID => Box::new(DeterministicProvider),
            _ => Box::new(DisabledProvider),
        }
    }

    /// Build the client exactly as the pipeline would for the same settings.
    pub fn client(&self) -> AiClient {
        let cfg = AiCfg {
            enabled: self.provider != crate::control::DISABLED_PROVIDER_ID,
            // Consent is per-job and explicit; it never widens the provider
            // table, and there is no external provider to widen it to.
            external_allowed: self.external_allowed,
            provider: self.provider.clone(),
            model: None,
            timeout_ms: self.timeout_ms,
            max_retries: self.max_retries,
            request_budget: self.request_budget,
            input_budget_bytes: self.max_input_bytes,
            thresholds: Thresholds {
                accept: self.confidence,
                review: self.confidence,
            },
            overwrite_validated: false,
            redact_fields: Vec::new(),
        };
        AiClient::new(cfg, self.provider())
    }
}

/// One suggestion as the control plane reports it. Carries no prompt, no
/// endpoint, no credential — the same rule as the rest of the API.
#[derive(Debug, Clone, Serialize)]
pub struct SuggestionView {
    pub field: String,
    pub suggested: String,
    pub confidence: f64,
    pub band: String,
    pub action: String,
    pub replaces_validated: bool,
    pub rationale: String,
}

/// Counters for one AI job. Every field is a count; none is a guess.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AiJobCounts {
    pub fields: u32,
    pub redacted: u32,
    pub suggestions: u32,
    pub accepted: u32,
    pub review: u32,
    pub rejected: u32,
    pub kept_validated: u32,
    pub invalid: u32,
}

/// The result of one AI job.
#[derive(Debug, Clone, Serialize)]
pub struct AiJobResult {
    pub state: String,
    pub task: String,
    pub provider: String,
    pub model: Option<String>,
    pub external: bool,
    pub external_allowed: bool,
    pub confidence: f64,
    pub counts: AiJobCounts,
    pub input_bytes: u64,
    pub latency_ms: Option<u64>,
    pub suggestions: Vec<SuggestionView>,
    /// Present when the round could not produce assistance: the provider
    /// state, verbatim. Never a stack trace, never an endpoint.
    pub reason: Option<String>,
}

impl AiJobResult {
    pub fn ok(&self) -> bool {
        self.state == AiState::Ok.as_str()
    }
}

const AI_SOURCE_FILE_CAP: u64 = 64 * 1024 * 1024;
const AI_SOURCE_LINE_CAP: usize = 1024 * 1024;
const AI_SOURCE_VALUE_CAP: usize = 2_048;
const AI_SOURCE_NAME_CAP: usize = 64;
const MAX_AI_RESOLVED_FIELDS: usize = 10_000;

/// A writer that retains only a bounded prefix while allowing a serializer to
/// finish normally. This avoids allocating a huge intermediate string for a
/// large JSON value in a source record.
struct PrefixWriter {
    bytes: Vec<u8>,
    cap: usize,
}
impl std::io::Write for PrefixWriter {
    fn write(&mut self, input: &[u8]) -> std::io::Result<usize> {
        let keep = self.cap.saturating_sub(self.bytes.len()).min(input.len());
        if keep == 0 && !input.is_empty() {
            return Err(std::io::Error::other("bounded prefix reached"));
        }
        self.bytes.extend_from_slice(&input[..keep]);
        Ok(keep)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn value_prefix(value: &Value, max_bytes: usize) -> String {
    if let Some(text) = value.as_str() {
        return clip(text, max_bytes);
    }
    let mut writer = PrefixWriter {
        bytes: Vec::with_capacity(max_bytes),
        cap: max_bytes,
    };
    let _ = serde_json::to_writer(&mut writer, value);
    clip(&String::from_utf8_lossy(&writer.bytes), max_bytes)
}

fn read_capped_line<R: std::io::BufRead>(
    reader: &mut R,
    line: &mut Vec<u8>,
    cap: usize,
) -> std::io::Result<usize> {
    line.clear();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(line.len());
        }
        let end = available
            .iter()
            .position(|b| *b == b'\n')
            .map_or(available.len(), |i| i + 1);
        if line.len().saturating_add(end) > cap {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "AI source line exceeds the 1 MiB cap",
            ));
        }
        let complete = available.get(end.saturating_sub(1)) == Some(&b'\n');
        line.extend_from_slice(&available[..end]);
        reader.consume(end);
        if complete {
            return Ok(line.len());
        }
    }
}

/// Turn a configured JSONL output into bounded [`AiField`]s. The file, line,
/// record, field, name and value limits are enforced before data is cloned
/// into the provider request.
pub fn fields_from_run_output(
    path: &Path,
    record_limit: usize,
    field_limit: usize,
) -> Result<Vec<AiField>, String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| "configured output path needs a filename".to_string())?;
    let file = crate::fsutil::open_within(parent, &parent.join(name), false)
        .map_err(|e| format!("cannot read the configured output file: {e}"))?;
    if file
        .metadata()
        .map_err(|e| format!("cannot stat the configured output file: {e}"))?
        .len()
        > AI_SOURCE_FILE_CAP
    {
        return Err("configured output exceeds the 64 MiB AI source file cap".into());
    }
    let mut reader = std::io::BufReader::new(crate::source_io::CappedReader::new(
        file,
        AI_SOURCE_FILE_CAP,
    ));
    let mut out: Vec<AiField> = Vec::new();
    let record_limit = record_limit.min(crate::control::MAX_AI_FIELDS);
    let field_limit = field_limit.min(crate::control::MAX_AI_FIELDS);
    let mut line = Vec::new();
    for index in 0..record_limit {
        let len = read_capped_line(&mut reader, &mut line, AI_SOURCE_LINE_CAP)
            .map_err(|e| format!("cannot read the output file: {e}"))?;
        if len == 0 {
            break;
        }
        let text = std::str::from_utf8(&line)
            .map_err(|e| format!("output line {} is not UTF-8: {e}", index + 1))?;
        if text.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(text.trim())
            .map_err(|e| format!("output line {} is not valid JSON: {e}", index + 1))?;
        let object = value
            .get("fields")
            .and_then(Value::as_object)
            .or_else(|| value.as_object());
        let Some(object) = object else {
            continue;
        };
        for (key, field_value) in object.iter().take(field_limit) {
            if out.len() >= MAX_AI_RESOLVED_FIELDS {
                return Err(format!(
                    "configured output exceeds the {MAX_AI_RESOLVED_FIELDS} resolved-field safety cap"
                ));
            }
            let value = value_prefix(field_value, AI_SOURCE_VALUE_CAP);
            out.push(AiField {
                name: clip(key, AI_SOURCE_NAME_CAP),
                deterministic: Some(value.clone()),
                value,
                redacted: false,
            });
        }
    }
    if out.is_empty() {
        return Err("the configured output holds no usable fields".to_string());
    }
    Ok(out)
}

/// Clip by UTF-8 byte count without splitting a code point.
fn clip(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Resolve the job's field set, enforcing the serialized AI budget while
/// fields are added rather than after an unbounded vector has been built.
pub fn resolve_fields(spec: &AiJobSpec, cfg: &Config) -> Result<Vec<AiField>, String> {
    match &spec.source {
        AiSource::RunOutput {
            record_limit,
            field_limit,
        } => {
            let files = crate::export_job::source_files(cfg).map_err(|e| e.message)?;
            let mut out = Vec::new();
            let mut serialized_size = AiRequest {
                task: spec.task,
                fields: Vec::new(),
                declared: Vec::new(),
            }
            .input_bytes();
            let mut budget_exceeded = false;
            let mut fields_exceeded = false;
            let visit_result = crate::source_io::visit(
                cfg,
                &files,
                (*record_limit).min(crate::control::MAX_AI_FIELDS) as u64,
                |value| {
                    let object = value
                        .get("fields")
                        .and_then(Value::as_object)
                        .or_else(|| value.as_object())
                        .ok_or_else(|| anyhow::anyhow!("record has no fields"))?;
                    for (key, field_value) in object
                        .iter()
                        .take((*field_limit).min(crate::control::MAX_AI_FIELDS))
                    {
                        if out.len() >= MAX_AI_RESOLVED_FIELDS {
                            fields_exceeded = true;
                            anyhow::bail!(
                                "resolved fields exceed the {MAX_AI_RESOLVED_FIELDS} safety cap"
                            );
                        }
                        let value = value_prefix(field_value, AI_SOURCE_VALUE_CAP);
                        let field = AiField {
                            name: clip(key, AI_SOURCE_NAME_CAP),
                            deterministic: Some(value.clone()),
                            value,
                            redacted: false,
                        };
                        let encoded_len = serde_json::to_vec(&field)?.len();
                        let extra = encoded_len.saturating_add(usize::from(!out.is_empty()));
                        let next_size = serialized_size.saturating_add(extra);
                        if next_size as u64 > spec.max_input_bytes {
                            budget_exceeded = true;
                            anyhow::bail!("resolved source exceeds the configured AI input cap");
                        }
                        serialized_size = next_size;
                        out.push(field);
                    }
                    Ok(())
                },
            );
            if budget_exceeded {
                return Err(format!(
                    "budget-exceeded: AI source exceeds the {} byte input cap; reduce the record or field limit",
                    spec.max_input_bytes
                ));
            }
            if fields_exceeded {
                return Err(format!(
                    "AI source exceeds the {MAX_AI_RESOLVED_FIELDS} resolved-field safety cap"
                ));
            }
            visit_result.map_err(|e| e.to_string())?;
            if out.is_empty() {
                return Err("the configured output holds no usable fields".into());
            }
            Ok(out)
        }
        AiSource::Inline { fields } => {
            if fields.len() > crate::control::MAX_AI_FIELDS {
                return Err(format!(
                    "inline AI source exceeds the {} field limit",
                    crate::control::MAX_AI_FIELDS
                ));
            }
            Ok(fields
                .iter()
                .map(|field| {
                    let value = clip(&field.value, crate::control::MAX_AI_FIELD_VALUE_BYTES);
                    AiField {
                        name: clip(&field.name, AI_SOURCE_NAME_CAP),
                        deterministic: field
                            .deterministic
                            .as_deref()
                            .map(|text| clip(text, crate::control::MAX_AI_FIELD_VALUE_BYTES)),
                        value,
                        redacted: false,
                    }
                })
                .collect())
        }
    }
}

/// Run one AI job. Never panics and never writes to the run output: the
/// outcome is applied to a scratch record so the counters are real while the
/// extracted data stays exactly as the deterministic pipeline left it.
pub fn run(spec: &AiJobSpec, cfg: &Config) -> AiJobResult {
    let provider_name = spec.provider.clone();
    let task = spec.task.as_str().to_string();
    let mut client = spec.client();
    let status = client.status();
    let external = status.external;

    let fields = match resolve_fields(spec, cfg) {
        Ok(f) => f,
        Err(e) => {
            let (state, reason) = match e.strip_prefix("budget-exceeded: ") {
                Some(message) => (AiState::BudgetExceeded.as_str(), message.to_string()),
                None => (AiState::Failed.as_str(), e),
            };
            return AiJobResult {
                state: state.to_string(),
                task,
                provider: provider_name,
                model: status.model,
                external,
                external_allowed: spec.external_allowed,
                confidence: spec.confidence,
                counts: AiJobCounts::default(),
                input_bytes: 0,
                latency_ms: None,
                suggestions: Vec::new(),
                reason: Some(reason),
            };
        }
    };

    let cfg_view = client.cfg().clone();
    let (fields, redaction) = crate::ai::redact(&cfg_view, fields);
    let req = AiRequest {
        task: spec.task,
        fields,
        declared: Vec::new(),
    };
    let input_bytes = req.input_bytes() as u64;
    if input_bytes > spec.max_input_bytes {
        return AiJobResult {
            state: AiState::BudgetExceeded.as_str().to_string(),
            task,
            provider: provider_name.clone(),
            model: status.model,
            external,
            external_allowed: spec.external_allowed,
            confidence: spec.confidence,
            counts: AiJobCounts {
                fields: redaction.fields,
                redacted: redaction.redacted,
                ..AiJobCounts::default()
            },
            input_bytes,
            latency_ms: None,
            suggestions: Vec::new(),
            reason: Some(format!(
                "request is {input_bytes} bytes, over the {} byte input cap",
                spec.max_input_bytes
            )),
        };
    }

    let started = std::time::Instant::now();
    let outcome = client.assist(&req);
    let latency_ms = Some(started.elapsed().as_millis() as u64);

    // Apply to a scratch record: counters come from the real merge rules, and
    // the run's data is never touched.
    let mut scratch = serde_json::Map::new();
    for f in &req.fields {
        if let Some(d) = &f.deterministic {
            scratch.insert(f.name.clone(), Value::String(d.clone()));
        }
    }
    let apply = crate::ai::apply_suggestions(&mut scratch, &outcome, &cfg_view);

    let mut counts = AiJobCounts {
        fields: redaction.fields,
        redacted: redaction.redacted,
        suggestions: outcome.suggestions.len() as u32,
        accepted: apply.accepted,
        review: apply.review,
        rejected: apply.rejected,
        kept_validated: apply.kept_validated,
        invalid: apply.invalid,
    };
    counts.suggestions = counts
        .suggestions
        .saturating_add(outcome.candidates.len() as u32)
        .saturating_add(outcome.normalizations.len() as u32);

    let suggestions = outcome
        .suggestions
        .iter()
        .map(|s| SuggestionView {
            field: s.field.clone(),
            suggested: s.suggested.as_str().to_string(),
            confidence: s.confidence.get(),
            band: s.confidence.band().as_str().to_string(),
            action: cfg_view
                .thresholds
                .decide(
                    s.confidence,
                    cfg_view.overwrite_validated,
                    s.replaces_validated,
                )
                .as_str()
                .to_string(),
            replaces_validated: s.replaces_validated,
            rationale: s.rationale.clone(),
        })
        .collect();

    AiJobResult {
        state: outcome.state.as_str().to_string(),
        task,
        provider: provider_name,
        model: outcome.provider.as_ref().and_then(|m| m.model.clone()),
        external,
        external_allowed: spec.external_allowed,
        confidence: spec.confidence,
        counts,
        input_bytes,
        latency_ms: outcome
            .provider
            .as_ref()
            .and_then(|m| m.latency_ms)
            .or(latency_ms),
        suggestions,
        reason: match outcome.state {
            AiState::Ok => None,
            other => Some(match other {
                AiState::Disabled => "ai assistance is disabled for this job".to_string(),
                AiState::Unconfigured => {
                    "the provider cannot be used in this deployment".to_string()
                }
                AiState::Timeout => "the provider timed out".to_string(),
                AiState::Failed => "the provider call failed".to_string(),
                AiState::BudgetExceeded => "the request budget was exhausted".to_string(),
                AiState::Invalid => "the provider returned an unusable payload".to_string(),
                AiState::Ok => String::new(),
            }),
        },
    }
}

#[cfg(test)]
mod tests;
