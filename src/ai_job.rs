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

/// Turn the configured output into bounded [`AiField`]s.
///
/// Only the run output the config declares is read, only `record_limit`
/// records are read, and only `field_limit` keys per record are taken. Values
/// are stringified and clipped, so a 50 MB field cannot ride into a provider
/// request.
pub fn fields_from_run_output(
    path: &Path,
    record_limit: usize,
    field_limit: usize,
) -> Result<Vec<AiField>, String> {
    use std::io::BufRead;
    let fh = std::fs::File::open(path)
        .map_err(|e| format!("cannot read the configured output file: {e}"))?;
    let mut out: Vec<AiField> = Vec::new();
    for (i, line) in std::io::BufReader::new(fh).lines().enumerate() {
        if i >= record_limit {
            break;
        }
        let line = line.map_err(|e| format!("cannot read the output file: {e}"))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(trimmed)
            .map_err(|e| format!("output line {} is not valid JSON: {e}", i + 1))?;
        let obj = match v.get("fields").and_then(Value::as_object) {
            Some(o) => o.clone(),
            None => match v.as_object() {
                Some(o) => o.clone(),
                None => continue,
            },
        };
        for (k, val) in obj.iter().take(field_limit) {
            let value = match val {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            out.push(AiField {
                name: k.clone(),
                value: clip(&value, 2_048),
                deterministic: Some(clip(&value, 2_048)),
                redacted: false,
            });
        }
    }
    if out.is_empty() {
        return Err("the configured output holds no usable fields".to_string());
    }
    Ok(out)
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Resolve the job's field set.
pub fn resolve_fields(spec: &AiJobSpec, cfg: &Config) -> Result<Vec<AiField>, String> {
    match &spec.source {
        AiSource::RunOutput {
            record_limit,
            field_limit,
        } => {
            let files = crate::export_job::source_files(cfg).map_err(|e| e.message)?;
            fields_from_run_output(&files[0], *record_limit, *field_limit)
        }
        AiSource::Inline { fields } => Ok(fields
            .iter()
            .map(|f| AiField {
                name: f.name.clone(),
                value: f.value.clone(),
                deterministic: f.deterministic.clone(),
                redacted: false,
            })
            .collect()),
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
            return AiJobResult {
                state: AiState::Failed.as_str().to_string(),
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
                reason: Some(e),
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
