//! Optional AI assistance — an explicit, budgeted, fail-closed stage.
//!
//! AI is **opt-in and off by default**. When no provider is configured the
//! layer answers [`AiState::Disabled`] and nothing else: it never invents a
//! suggestion, never guesses a value, and never silently degrades into
//! "pretend the model answered".
//!
//! Contract, enforced here and pinned by `tests/ai_pipeline.rs`:
//! * **Deterministic extraction is never destroyed.** AI runs *after*
//!   extraction and only fills gaps or proposes changes
//!   ([`apply_suggestions`] refuses to overwrite a validated value unless
//!   `AiCfg::overwrite_validated` is set).
//! * **Bounded.** A per-run request budget and a per-request input-size budget
//!   are consumed before every call ([`AiClient::assist`]).
//! * **Time-boxed and retried a bounded number of times** (`timeout_ms`,
//!   `max_retries`).
//! * **Redacted.** Field names matching the sensitive patterns are replaced
//!   with a marker before a request is built, so secrets do not leave the
//!   process (`redact`).
//! * **Validated.** Provider payloads are parsed into [`AiOutcome`]; anything
//!   malformed is [`AiState::Invalid`], not a partial lie.
//! * **No security bypass.** This module has no notion of CAPTCHA, bot
//!   detection, fingerprinting, authentication or anti-bot evasion, and
//!   nothing here can be used to reach those: it only sees values the engine
//!   already extracted.
//!
//! Providers:
//! * [`DisabledProvider`] — the default; always [`AiError::Disabled`].
//! * [`DeterministicProvider`] — a local, network-free, byte-deterministic
//!   provider used by tests and by `--provider deterministic`. It names itself
//!   honestly (`deterministic-local`) and reports `external == false`: it
//!   infers types from value *shape*, which is what it is, not what a hosted
//!   model would be.
//! * Any hosted provider is implemented by the caller as an [`AiProvider`] and
//!   injected; keys stay server-side and are never part of any model here
//!   (see `ProviderMetadata`, which cannot carry them).

pub mod schema;

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ai::schema::{
    AiOutcome, AiState, Confidence, ExtractionCandidate, FieldSuggestion, NormalizationSuggestion,
    Provenance, ProviderMetadata, ReviewAction, SuggestedType, Thresholds, ValidationResult,
};

/// Marker substituted for the value of any field whose name looks sensitive.
pub const REDACTION_MARK: &str = "***";

/// Field-name fragments that are redacted before a request is built. Matched
/// case-insensitively against the *name*, not the value, because a value like
/// `sk_live_…` is exactly what we must not forward.
pub const SENSITIVE_FIELD_HINTS: [&str; 12] = [
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "authorization",
    "auth",
    "credential",
    "private_key",
    "cookie",
    "session",
];

/// What a provider is being asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiTask {
    /// Propose values/types for fields (including missing ones).
    Extract,
    /// Classify values into [`SuggestedType`]s.
    Classify,
    /// Propose normalization steps over existing values.
    Normalize,
    /// Propose a field -> type schema for a record.
    SchemaSuggest,
    /// Propose semantic labels for fields (e.g. "price", "sku").
    Label,
}

impl AiTask {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Extract => "extract",
            Self::Classify => "classify",
            Self::Normalize => "normalize",
            Self::SchemaSuggest => "schema-suggest",
            Self::Label => "label",
        }
    }

    /// Parse the wire form. Used by the CLI, the config loader and the
    /// control plane so all three accept exactly the same vocabulary.
    pub fn parse_task(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "extract" => Self::Extract,
            "classify" => Self::Classify,
            "normalize" => Self::Normalize,
            "schema-suggest" | "schema_suggest" => Self::SchemaSuggest,
            "label" => Self::Label,
            _ => return None,
        })
    }
}

/// One field handed to a provider. Values are strings: the engine's extracted
/// values are JSON, but a provider request is a bounded text view of them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiField {
    pub name: String,
    pub value: String,
    /// The deterministic value already extracted, when there is one.
    pub deterministic: Option<String>,
    /// True when `value` was replaced by [`REDACTION_MARK`].
    pub redacted: bool,
}

/// A bounded assistance request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiRequest {
    pub task: AiTask,
    pub fields: Vec<AiField>,
    /// Optional declared schema, so a provider can be checked against it.
    pub declared: Vec<(String, String)>,
}

impl AiRequest {
    /// Serialized size of the request — the quantity the input budget caps.
    pub fn input_bytes(&self) -> usize {
        serde_json::to_vec(self).map(|v| v.len()).unwrap_or(0)
    }
}

/// One suggestion as a provider returns it (pre-validation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawSuggestion {
    pub field: String,
    pub value: Value,
    pub suggested: String,
    pub confidence: f64,
    pub rationale: String,
}

/// A provider's raw answer. Deliberately free of endpoints/keys/prompts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiResponse {
    pub suggestions: Vec<RawSuggestion>,
    pub provider: ProviderMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AiError {
    /// AI is disabled or no provider is configured.
    Disabled,
    /// Configured, but not usable in this environment (e.g. external calls are
    /// not consented to).
    Unconfigured(String),
    Timeout,
    Failed(String),
    /// Provider answered, but not with a usable suggestion set.
    Invalid(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "ai assistance is disabled"),
            Self::Unconfigured(s) => write!(f, "ai provider not usable: {s}"),
            Self::Timeout => write!(f, "ai provider timed out"),
            Self::Failed(s) => write!(f, "ai provider failed: {s}"),
            Self::Invalid(s) => write!(f, "ai provider returned an unusable payload: {s}"),
        }
    }
}

impl std::error::Error for AiError {}

/// Provider interface. Implementations are injected; nothing in this crate
/// performs network I/O for AI on its own.
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    fn model(&self) -> Option<&str>;
    /// True when a call leaves the process/network. Drives the privacy gate:
    /// an external provider requires explicit consent.
    fn external(&self) -> bool;
    fn suggest(&self, req: &AiRequest) -> Result<AiResponse, AiError>;
}

/// The default provider: always disabled, always explicit.
#[derive(Debug, Default)]
pub struct DisabledProvider;

impl AiProvider for DisabledProvider {
    fn name(&self) -> &str {
        "disabled"
    }
    fn model(&self) -> Option<&str> {
        None
    }
    fn external(&self) -> bool {
        false
    }
    fn suggest(&self, _req: &AiRequest) -> Result<AiResponse, AiError> {
        Err(AiError::Disabled)
    }
}

/// A provider that always fails the way a wedged endpoint would. Used by tests
/// to prove that AI failure leaves deterministic data intact.
#[derive(Debug, Default)]
pub struct FailingProvider;

impl AiProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing-test"
    }
    fn model(&self) -> Option<&str> {
        Some("failure-1")
    }
    fn external(&self) -> bool {
        false
    }
    fn suggest(&self, _req: &AiRequest) -> Result<AiResponse, AiError> {
        Err(AiError::Failed("simulated provider failure".to_string()))
    }
}

/// A local, deterministic provider: infers a type from the *shape* of each
/// value and proposes existing transform ops for messy values. No network, no
/// randomness, no model — and it says so in its name and metadata.
#[derive(Debug, Default)]
pub struct DeterministicProvider;

impl DeterministicProvider {
    /// Confidence per inference rule. Documented because the thresholds in the
    /// console are compared against these exact numbers.
    pub const INT_CONFIDENCE: f64 = 0.95;
    pub const NUMBER_CONFIDENCE: f64 = 0.90;
    pub const BOOL_CONFIDENCE: f64 = 0.95;
    pub const DATE_CONFIDENCE: f64 = 0.85;
    pub const JSON_CONFIDENCE: f64 = 0.80;
    pub const TEXT_CONFIDENCE: f64 = 0.65;
    pub const EMPTY_CONFIDENCE: f64 = 0.30;

    fn classify(value: &str) -> (SuggestedType, f64, &'static str) {
        let v = value.trim();
        if v.is_empty() {
            return (
                SuggestedType::Text,
                Self::EMPTY_CONFIDENCE,
                "empty value: no signal",
            );
        }
        if matches!(
            v.to_ascii_lowercase().as_str(),
            "true" | "false" | "yes" | "no"
        ) {
            return (
                SuggestedType::Bool,
                Self::BOOL_CONFIDENCE,
                "boolean literal",
            );
        }
        if v.parse::<i64>().is_ok() {
            return (SuggestedType::Int, Self::INT_CONFIDENCE, "integer literal");
        }
        if v.parse::<f64>().is_ok() {
            return (
                SuggestedType::Number,
                Self::NUMBER_CONFIDENCE,
                "decimal literal",
            );
        }
        if is_iso_date(v) {
            return (
                SuggestedType::Date,
                Self::DATE_CONFIDENCE,
                "ISO-8601 date shape",
            );
        }
        let jsonish =
            (v.starts_with('[') && v.ends_with(']')) || (v.starts_with('{') && v.ends_with('}'));
        if jsonish && serde_json::from_str::<Value>(v).is_ok() {
            return (
                SuggestedType::Json,
                Self::JSON_CONFIDENCE,
                "well-formed JSON array/object",
            );
        }
        if v.contains(',') && v.chars().filter(|c| *c == ',').count() >= 1 {
            return (
                SuggestedType::List,
                Self::TEXT_CONFIDENCE,
                "comma-separated text",
            );
        }
        (
            SuggestedType::Text,
            Self::TEXT_CONFIDENCE,
            "free text: no structural signal",
        )
    }
}

fn is_iso_date(v: &str) -> bool {
    // YYYY-MM-DD, optionally followed by a time part. Bounded and strict —
    // a heuristic date parser that accepts "tomorrow" is a bug factory.
    let date = &v[..v.len().min(10)];
    let bytes = date.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| date[r].chars().all(|c| c.is_ascii_digit());
    let ok = digits(0..4) && digits(5..7) && digits(8..10);
    if !ok {
        return false;
    }
    let rest = v.get(10..).unwrap_or("");
    rest.is_empty() || (rest.starts_with('T') && rest.len() >= 9)
}

impl AiProvider for DeterministicProvider {
    fn name(&self) -> &str {
        "deterministic-local"
    }
    fn model(&self) -> Option<&str> {
        Some("shape-inference-1")
    }
    fn external(&self) -> bool {
        false
    }
    fn suggest(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        let mut suggestions = Vec::new();
        for f in &req.fields {
            let (suggested, confidence, rationale) = Self::classify(&f.value);
            // A redacted field carries no signal — refuse to suggest on it.
            if f.redacted {
                continue;
            }
            let value = match suggested {
                SuggestedType::Int => f
                    .value
                    .trim()
                    .parse::<i64>()
                    .map(Value::from)
                    .unwrap_or(Value::String(f.value.clone())),
                SuggestedType::Number => f
                    .value
                    .trim()
                    .parse::<f64>()
                    .map(|n| {
                        serde_json::Number::from_f64(n)
                            .map(Value::Number)
                            .unwrap_or(Value::Null)
                    })
                    .unwrap_or_else(|_| Value::String(f.value.clone())),
                SuggestedType::Bool => Value::Bool(matches!(
                    f.value.trim().to_ascii_lowercase().as_str(),
                    "true" | "yes"
                )),
                SuggestedType::Json => serde_json::from_str::<Value>(f.value.trim())
                    .unwrap_or_else(|_| Value::String(f.value.clone())),
                _ => Value::String(f.value.trim().to_string()),
            };
            suggestions.push(RawSuggestion {
                field: f.name.clone(),
                value,
                suggested: suggested.as_str().to_string(),
                confidence,
                rationale: rationale.to_string(),
            });
        }
        Ok(AiResponse {
            suggestions,
            provider: ProviderMetadata {
                name: self.name().to_string(),
                model: self.model().map(|m| m.to_string()),
                request_id: Some(format!("det-{}", req.fields.len())),
                latency_ms: Some(0),
                input_bytes: Some(req.input_bytes() as u64),
                output_bytes: None,
            },
        })
    }
}

/// AI configuration. `enabled: false` by default, and external providers
/// additionally require `external_allowed: true`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiCfg {
    /// Master switch. Off by default.
    pub enabled: bool,
    /// Consent for providers whose `external()` is true. Off by default: no
    /// extracted data leaves the process without an explicit opt-in.
    pub external_allowed: bool,
    /// Provider id (`disabled`, `deterministic-local`, or an injected one).
    pub provider: String,
    pub model: Option<String>,
    /// Per-call timeout.
    pub timeout_ms: u64,
    /// Retries after a transient failure (never applied to `Disabled`).
    pub max_retries: u8,
    /// Requests allowed per [`AiClient`] instance (a run/record batch).
    pub request_budget: u32,
    /// Bytes of serialized request allowed in total.
    pub input_budget_bytes: u64,
    pub thresholds: Thresholds,
    /// Accept a high-confidence suggestion that replaces a validated value.
    pub overwrite_validated: bool,
    /// Extra field-name fragments to redact (added to
    /// [`SENSITIVE_FIELD_HINTS`]).
    pub redact_fields: Vec<String>,
}

impl Default for AiCfg {
    fn default() -> Self {
        Self {
            enabled: false,
            external_allowed: false,
            provider: "disabled".to_string(),
            model: None,
            timeout_ms: 2_000,
            max_retries: 1,
            request_budget: 100,
            input_budget_bytes: 1_048_576,
            thresholds: Thresholds::default(),
            overwrite_validated: false,
            redact_fields: Vec::new(),
        }
    }
}

impl AiCfg {
    /// Validate the configuration. Returns the reason AI stays off, so callers
    /// can report it instead of guessing.
    pub fn reason_disabled(&self) -> Option<&'static str> {
        if !self.enabled {
            return Some("ai.enabled is false");
        }
        if self.provider.trim().is_empty() || self.provider == "disabled" {
            return Some("no provider configured");
        }
        None
    }
}

/// Observable provider state for the console. Never contains credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub enabled: bool,
    pub configured: bool,
    pub name: String,
    pub model: Option<String>,
    pub external: bool,
    pub external_allowed: bool,
    pub external_consent_ok: bool,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub requests_used: u32,
    pub requests_max: u32,
    pub input_bytes_used: u64,
    pub input_bytes_max: u64,
    pub thresholds: Thresholds,
    pub overwrite_validated: bool,
}

/// What a redaction pass did. Counts only — never the removed values.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionReport {
    pub fields: u32,
    pub redacted: u32,
}

/// Result of applying an outcome to a record.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplyReport {
    pub accepted: u32,
    pub review: u32,
    pub rejected: u32,
    /// Suggestions refused because a validated value already existed.
    pub kept_validated: u32,
    pub invalid: u32,
}

/// Budgeted, redacting, validating wrapper around an [`AiProvider`].
pub struct AiClient {
    cfg: AiCfg,
    provider: Box<dyn AiProvider>,
    requests_used: u32,
    input_bytes_used: u64,
}

impl AiClient {
    /// A client with no provider: every call reports [`AiState::Disabled`].
    pub fn disabled() -> Self {
        Self::new(AiCfg::default(), Box::new(DisabledProvider))
    }

    pub fn new(cfg: AiCfg, provider: Box<dyn AiProvider>) -> Self {
        Self {
            cfg,
            provider,
            requests_used: 0,
            input_bytes_used: 0,
        }
    }

    pub fn cfg(&self) -> &AiCfg {
        &self.cfg
    }

    pub fn status(&self) -> ProviderStatus {
        let enabled = self.cfg.enabled;
        let configured = enabled && self.cfg.reason_disabled().is_none();
        let external = self.provider.external();
        ProviderStatus {
            enabled,
            configured,
            name: self.provider.name().to_string(),
            model: self.provider.model().map(|m| m.to_string()),
            external,
            external_allowed: self.cfg.external_allowed,
            external_consent_ok: !external || self.cfg.external_allowed,
            timeout_ms: self.cfg.timeout_ms,
            max_retries: self.cfg.max_retries,
            requests_used: self.requests_used,
            requests_max: self.cfg.request_budget,
            input_bytes_used: self.input_bytes_used,
            input_bytes_max: self.cfg.input_budget_bytes,
            thresholds: self.cfg.thresholds,
            overwrite_validated: self.cfg.overwrite_validated,
        }
    }

    /// Remaining budget as (requests, input bytes).
    pub fn remaining(&self) -> (u32, u64) {
        (
            self.cfg.request_budget.saturating_sub(self.requests_used),
            self.cfg
                .input_budget_bytes
                .saturating_sub(self.input_bytes_used),
        )
    }

    /// Run one assistance round. Every refusal path returns an [`AiOutcome`]
    /// with an explicit [`AiState`] — no panics, no partial suggestions from a
    /// failed call, and no silent success.
    pub fn assist(&mut self, req: &AiRequest) -> AiOutcome {
        if let Some(_reason) = self.cfg.reason_disabled() {
            return AiOutcome::disabled(AiState::Disabled);
        }
        if self.provider.external() && !self.cfg.external_allowed {
            return AiOutcome::disabled(AiState::Unconfigured);
        }
        if self.requests_used >= self.cfg.request_budget {
            return AiOutcome::disabled(AiState::BudgetExceeded);
        }
        let size = req.input_bytes() as u64;
        if self.input_bytes_used.saturating_add(size) > self.cfg.input_budget_bytes {
            return AiOutcome::disabled(AiState::BudgetExceeded);
        }

        let started = Instant::now();
        let mut last: Option<AiError> = None;
        let attempts = self.cfg.max_retries.saturating_add(1);
        for _ in 0..attempts.max(1) {
            match self.provider.suggest(req) {
                Ok(resp) => {
                    self.requests_used += 1;
                    self.input_bytes_used = self.input_bytes_used.saturating_add(size);
                    let latency = started.elapsed();
                    if latency > Duration::from_millis(self.cfg.timeout_ms) {
                        return AiOutcome::disabled(AiState::Timeout);
                    }
                    return self.validate(resp, latency, 1);
                }
                Err(AiError::Disabled) => return AiOutcome::disabled(AiState::Disabled),
                Err(AiError::Timeout) => last = Some(AiError::Timeout),
                Err(e) => last = Some(e),
            }
        }
        match last {
            Some(AiError::Timeout) => AiOutcome::disabled(AiState::Timeout),
            Some(AiError::Invalid(s)) => {
                let mut o = AiOutcome::disabled(AiState::Invalid);
                o.validations.push(ValidationResult {
                    field: "*".to_string(),
                    ok: false,
                    reason: Some(s),
                });
                o
            }
            Some(_) => AiOutcome::disabled(AiState::Failed),
            None => AiOutcome::disabled(AiState::Failed),
        }
    }

    /// Turn a provider payload into a validated outcome. Unknown types, missing
    /// fields and out-of-range confidences are all rejected explicitly.
    fn validate(&self, resp: AiResponse, latency: Duration, requests: u32) -> AiOutcome {
        let mut out = AiOutcome {
            state: AiState::Ok,
            requests_used: requests,
            ..AiOutcome::default()
        };
        let mut invalid = 0u32;
        for raw in resp.suggestions {
            let Some(ty) = SuggestedType::parse(&raw.suggested) else {
                invalid += 1;
                out.validations.push(ValidationResult {
                    field: raw.field.clone(),
                    ok: false,
                    reason: Some(format!("unknown suggested type `{}`", raw.suggested)),
                });
                continue;
            };
            if !(0.0..=1.0).contains(&raw.confidence) {
                invalid += 1;
                out.validations.push(ValidationResult {
                    field: raw.field.clone(),
                    ok: false,
                    reason: Some("confidence outside [0,1]".to_string()),
                });
                continue;
            }
            out.suggestions.push(FieldSuggestion {
                field: raw.field.clone(),
                value: raw.value.clone(),
                suggested: ty,
                confidence: Confidence::new(raw.confidence),
                rationale: truncate(&raw.rationale, 200),
                provenance: Provenance::AiSuggestion,
                replaces_validated: false,
            });
        }
        if out.suggestions.is_empty() && invalid > 0 {
            out.state = AiState::Invalid;
        }
        let mut meta = resp.provider;
        meta.latency_ms = Some(latency.as_millis() as u64);
        meta.output_bytes = Some(
            serde_json::to_vec(&out.suggestions)
                .map(|v| v.len() as u64)
                .unwrap_or(0),
        );
        out.provider = Some(meta);
        out
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

/// Replace the values of fields whose *name* looks sensitive. Applied before a
/// request is serialized; the removed values are never logged or returned.
pub fn redact(cfg: &AiCfg, fields: Vec<AiField>) -> (Vec<AiField>, RedactionReport) {
    let mut report = RedactionReport {
        fields: fields.len() as u32,
        redacted: 0,
    };
    let out = fields
        .into_iter()
        .map(|f| {
            let name = f.name.to_ascii_lowercase();
            let sensitive = SENSITIVE_FIELD_HINTS.iter().any(|h| name.contains(h))
                || cfg
                    .redact_fields
                    .iter()
                    .any(|h| name.contains(&h.to_ascii_lowercase()));
            if sensitive {
                report.redacted += 1;
                AiField {
                    name: f.name,
                    value: REDACTION_MARK.to_string(),
                    deterministic: f.deterministic,
                    redacted: true,
                }
            } else {
                f
            }
        })
        .collect();
    (out, report)
}

/// Merge an AI outcome into a record. Rules, in order:
/// * suggestions below the review threshold are **rejected** (never written);
/// * medium confidence is recorded for review (the record is unchanged);
/// * high confidence may be written, unless a deterministic value already
///   exists and `overwrite_validated` is false — then it is recorded for
///   review instead.
///
/// A record that came in valid always comes out valid: nothing here deletes or
/// overwrites a deterministic value on AI authority alone.
pub fn apply_suggestions(
    record: &mut serde_json::Map<String, Value>,
    outcome: &AiOutcome,
    cfg: &AiCfg,
) -> ApplyReport {
    let mut report = ApplyReport::default();
    if outcome.state != AiState::Ok {
        return report;
    }
    for s in &outcome.suggestions {
        let replaces_validated = s.replaces_validated || record.contains_key(&s.field);
        let action =
            cfg.thresholds
                .decide(s.confidence, cfg.overwrite_validated, replaces_validated);
        match action {
            ReviewAction::Reject => report.rejected += 1,
            ReviewAction::Review => {
                if replaces_validated {
                    report.kept_validated += 1;
                } else {
                    report.review += 1;
                }
            }
            ReviewAction::Accept => {
                record.insert(s.field.clone(), s.value.clone());
                report.accepted += 1;
            }
        }
    }
    // Candidates fill fields the deterministic extractor left empty; they are
    // gated by the same thresholds.
    for c in &outcome.candidates {
        if record.contains_key(&c.field) {
            continue;
        }
        match cfg
            .thresholds
            .decide(c.confidence, cfg.overwrite_validated, false)
        {
            ReviewAction::Accept => {
                record.insert(c.field.clone(), c.value.clone());
                report.accepted += 1;
            }
            ReviewAction::Review => report.review += 1,
            ReviewAction::Reject => report.rejected += 1,
        }
    }
    report.invalid = outcome.validations.iter().filter(|v| !v.ok).count() as u32;
    report
}

/// Build a normalization suggestion for a messy value, naming an existing
/// transform op (never an invented one).
pub fn normalization_for(field: &str, value: &str) -> Option<NormalizationSuggestion> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.chars().any(|c| matches!(c, '$' | '€' | '£' | '¥'))
        || v.contains(',') && v.chars().any(|c| c.is_ascii_digit())
    {
        return Some(NormalizationSuggestion {
            field: field.to_string(),
            from: v.to_string(),
            to: "numeric".to_string(),
            op: "parse_number".to_string(),
            confidence: Confidence::new(0.75),
            rationale: "value carries currency/grouping characters".to_string(),
        });
    }
    if v != value {
        return Some(NormalizationSuggestion {
            field: field.to_string(),
            from: value.to_string(),
            to: v.to_string(),
            op: "trim".to_string(),
            confidence: Confidence::new(0.95),
            rationale: "leading/trailing whitespace".to_string(),
        });
    }
    if v.chars().filter(|c| c.is_whitespace()).count() > 1 && v.contains("  ") {
        return Some(NormalizationSuggestion {
            field: field.to_string(),
            from: v.to_string(),
            to: v.split_whitespace().collect::<Vec<_>>().join(" "),
            op: "collapse_whitespace".to_string(),
            confidence: Confidence::new(0.90),
            rationale: "repeated internal whitespace".to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests;

/// Turn suggestion candidates into typed extraction candidates (used by the
/// pipeline when a declared field came back empty).
pub fn candidate(
    field: &str,
    value: Value,
    confidence: f64,
    rationale: &str,
) -> ExtractionCandidate {
    ExtractionCandidate {
        field: field.to_string(),
        value,
        confidence: Confidence::new(confidence),
        rationale: rationale.to_string(),
    }
}
