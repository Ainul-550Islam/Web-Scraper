//! Typed AI *assistance* models.
//!
//! These types are the contract between three parties that must never be
//! confused with each other:
//!
//! 1. **deterministic extraction** — what the engine extracted from the page
//!    (CSS/JSONPath). Authoritative by construction.
//! 2. **AI suggestion** — what a provider *proposes*. Always labelled, always
//!    carrying a confidence and a rationale, never authoritative.
//! 3. **validated value** — a deterministic (or human-confirmed) value that an
//!    AI suggestion must not overwrite unless policy explicitly allows it.
//!
//! Nothing in this module can express "the model said so, therefore it is
//! true": every suggestion carries [`Provenance`], and the console renders
//! `AI SUGGESTION` for exactly one of its variants.
//!
//! Provider metadata is deliberately narrow — provider name, model id, request
//! id, latency, byte counts. Prompts, keys, endpoints and raw provider payloads
//! are **not** modelled here, so they cannot leak into a delivered dataset, a
//! log line, or the browser.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::schema::DeclaredType;

/// Where a value came from. This is the single most important field in the AI
/// surface: it is what stops an assisted value from being mistaken for a
/// validated one downstream (sinks, exports, console labels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provenance {
    /// Deterministic extraction or transform produced this value.
    Deterministic,
    /// A provider proposed it. Never authoritative.
    AiSuggestion,
    /// An AI suggestion a human (or an explicit policy) accepted.
    AiConfirmed,
}

impl Provenance {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Deterministic => "deterministic",
            Self::AiSuggestion => "ai-suggestion",
            Self::AiConfirmed => "ai-confirmed",
        }
    }

    /// The console's required label: an assisted value must say so on screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Deterministic => "VALIDATED",
            Self::AiSuggestion => "AI SUGGESTION",
            Self::AiConfirmed => "AI CONFIRMED",
        }
    }

    pub fn is_ai(self) -> bool {
        matches!(self, Self::AiSuggestion | Self::AiConfirmed)
    }
}

/// Confidence in `[0,1]`, clamped on construction so a provider returning `7.5`
/// or `-1` cannot skew thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence(f64);

impl Confidence {
    pub const HIGH: f64 = 0.90;
    pub const MEDIUM: f64 = 0.60;

    pub fn new(v: f64) -> Self {
        Self(if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) })
    }

    pub fn get(self) -> f64 {
        self.0
    }

    pub fn band(self) -> Band {
        Band::of(self.0)
    }

    /// Two decimals: enough to display, stable across runs and platforms.
    pub fn as_display(self) -> String {
        format!("{:.2}", self.0)
    }
}

/// Confidence bands, mapped to the review workflow in the module docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Band {
    /// Above [`Confidence::HIGH`]: may be auto-accepted, if configured so.
    High,
    /// Between MEDIUM and HIGH: marked for review.
    Medium,
    /// Below MEDIUM: rejected (or quarantined, per schema enforcement).
    Low,
}

impl Band {
    pub fn of(v: f64) -> Self {
        if v >= Confidence::HIGH {
            Self::High
        } else if v >= Confidence::MEDIUM {
            Self::Medium
        } else {
            Self::Low
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
        }
    }
}

/// A suggested field type, convertible to (and from) the schema layer's
/// [`DeclaredType`] so a suggestion can be checked against a declared schema
/// instead of inventing a parallel type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuggestedType {
    Text,
    Number,
    Int,
    Bool,
    Date,
    List,
    Json,
}

impl SuggestedType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Int => "int",
            Self::Bool => "bool",
            Self::Date => "date",
            Self::List => "list",
            Self::Json => "json",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "text" | "string" => Self::Text,
            "number" | "float" | "double" => Self::Number,
            "int" | "integer" => Self::Int,
            "bool" | "boolean" => Self::Bool,
            "date" | "datetime" => Self::Date,
            "list" | "array" => Self::List,
            "json" | "object" => Self::Json,
            _ => return None,
        })
    }

    pub fn to_declared(self) -> DeclaredType {
        match self {
            Self::Text => DeclaredType::Text,
            Self::Number => DeclaredType::Number,
            Self::Int => DeclaredType::Int,
            Self::Bool => DeclaredType::Bool,
            Self::Date => DeclaredType::Date,
            Self::List => DeclaredType::List,
            Self::Json => DeclaredType::Json,
        }
    }

    pub fn from_declared(t: DeclaredType) -> Self {
        match t {
            DeclaredType::Text => Self::Text,
            DeclaredType::Number => Self::Number,
            DeclaredType::Int => Self::Int,
            DeclaredType::Bool => Self::Bool,
            DeclaredType::Date => Self::Date,
            DeclaredType::List => Self::List,
            DeclaredType::Json => Self::Json,
        }
    }
}

/// One proposed field value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldSuggestion {
    pub field: String,
    pub value: Value,
    pub suggested: SuggestedType,
    pub confidence: Confidence,
    /// Short, human-readable justification. Never contains prompt text.
    pub rationale: String,
    pub provenance: Provenance,
    /// True when a deterministic value already exists for this field and the
    /// suggestion would replace it (gated by `AiCfg::overwrite_validated`).
    pub replaces_validated: bool,
}

impl FieldSuggestion {
    pub fn to_json(&self) -> Value {
        json!({
            "field": self.field,
            "value": self.value,
            "suggested": self.suggested.as_str(),
            "confidence": self.confidence.get(),
            "band": self.confidence.band().as_str(),
            "rationale": self.rationale,
            "provenance": self.provenance.as_str(),
            "label": self.provenance.label(),
            "replaces_validated": self.replaces_validated,
        })
    }
}

/// A candidate extraction for a field the deterministic extractor could not
/// fill (missing selector, changed markup, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractionCandidate {
    pub field: String,
    pub value: Value,
    pub confidence: Confidence,
    pub rationale: String,
}

/// A proposed normalization step (e.g. "strip the currency symbol", "parse
/// `1,234` as 1234"). Applied only through the existing transform pipeline —
/// the AI layer never rewrites values behind the transform layer's back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizationSuggestion {
    pub field: String,
    pub from: String,
    pub to: String,
    /// Name of an existing `transform::Transform` op, or "none".
    pub op: String,
    pub confidence: Confidence,
    pub rationale: String,
}

/// Result of checking one suggestion against the declared schema (when there
/// is one). `ok == false` never destroys the deterministic value — it routes
/// the record through the existing quarantine path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationResult {
    pub field: String,
    pub ok: bool,
    pub reason: Option<String>,
}

/// What the caller should do with a suggestion, given configured thresholds
/// and whether a validated value is already present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewAction {
    Accept,
    Review,
    Reject,
}

impl ReviewAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accept => "accept",
            Self::Review => "review",
            Self::Reject => "reject",
        }
    }
}

/// Confidence thresholds. Defaults match the documented policy: high ⇒ accept
/// (when auto-accept is enabled), medium ⇒ review, low ⇒ reject.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// `>= accept` ⇒ [`ReviewAction::Accept`] when auto-accept is on.
    pub accept: f64,
    /// `>= review` ⇒ [`ReviewAction::Review`]; below ⇒ reject.
    pub review: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            accept: Confidence::HIGH,
            review: Confidence::MEDIUM,
        }
    }
}

impl Thresholds {
    /// Decide what happens to a suggestion. Order of the rules is the contract:
    /// a low-confidence suggestion is rejected even when auto-accept is on, and
    /// a suggestion that would replace a validated value is only accepted when
    /// `overwrite_validated` is explicitly true.
    pub fn decide(
        &self,
        c: Confidence,
        overwrite_validated: bool,
        replaces_validated: bool,
    ) -> ReviewAction {
        let v = c.get();
        if v < self.review {
            return ReviewAction::Reject;
        }
        if v >= self.accept {
            if replaces_validated && !overwrite_validated {
                return ReviewAction::Review;
            }
            return ReviewAction::Accept;
        }
        ReviewAction::Review
    }
}

/// Minimal, non-sensitive metadata about the provider call that produced a
/// result. Deliberately excludes endpoints, keys, prompts and raw payloads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderMetadata {
    pub name: String,
    pub model: Option<String>,
    /// Provider-supplied request id, when it returned one.
    pub request_id: Option<String>,
    pub latency_ms: Option<u64>,
    pub input_bytes: Option<u64>,
    pub output_bytes: Option<u64>,
}

impl ProviderMetadata {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "model": self.model,
            "request_id": self.request_id,
            "latency_ms": self.latency_ms,
            "input_bytes": self.input_bytes,
            "output_bytes": self.output_bytes,
        })
    }
}

/// Why an AI request produced nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiState {
    /// No provider configured / AI disabled. The only honest answer for a
    /// default install.
    #[default]
    Disabled,
    /// Configured, but the provider could not be used (no external consent, …).
    Unconfigured,
    Ok,
    Timeout,
    /// Provider call failed after the configured retries.
    Failed,
    /// Request or input-byte budget exhausted.
    BudgetExceeded,
    /// Provider answered, but the payload was not a usable suggestion set.
    Invalid,
}

impl AiState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Unconfigured => "unconfigured",
            Self::Ok => "ok",
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::BudgetExceeded => "budget-exceeded",
            Self::Invalid => "invalid",
        }
    }
}

/// One AI assistance round for one record: everything the pipeline and the
/// console need, and nothing else.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AiOutcome {
    pub state: AiState,
    pub suggestions: Vec<FieldSuggestion>,
    pub candidates: Vec<ExtractionCandidate>,
    pub normalizations: Vec<NormalizationSuggestion>,
    pub validations: Vec<ValidationResult>,
    pub provider: Option<ProviderMetadata>,
    /// Requests consumed by this round.
    pub requests_used: u32,
}

impl AiOutcome {
    pub fn disabled(reason: AiState) -> Self {
        Self {
            state: reason,
            ..Self::default()
        }
    }

    /// True only when the round actually produced usable assistance.
    pub fn has_suggestions(&self) -> bool {
        !self.suggestions.is_empty()
            || !self.candidates.is_empty()
            || !self.normalizations.is_empty()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "state": self.state.as_str(),
            "suggestions": self.suggestions.iter().map(|s| s.to_json()).collect::<Vec<_>>(),
            "candidates": self.candidates,
            "normalizations": self.normalizations,
            "validations": self.validations,
            "provider": self.provider.as_ref().map(|p| p.to_json()),
            "requests_used": self.requests_used,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn confidence_clamps_and_bands() {
        assert_eq!(Confidence::new(7.5).get(), 1.0);
        assert_eq!(Confidence::new(-2.0).get(), 0.0);
        assert!(Confidence::new(f64::NAN).get() == 0.0);
        assert_eq!(Confidence::new(0.95).band(), Band::High);
        assert_eq!(Confidence::new(0.72).band(), Band::Medium);
        assert_eq!(Confidence::new(0.10).band(), Band::Low);
        assert_eq!(Confidence::new(0.9).as_display(), "0.90");
    }

    #[test]
    fn thresholds_never_accept_low_confidence() {
        let t = Thresholds::default();
        assert_eq!(
            t.decide(Confidence::new(0.2), true, false),
            ReviewAction::Reject
        );
        assert_eq!(
            t.decide(Confidence::new(0.7), true, false),
            ReviewAction::Review
        );
        assert_eq!(
            t.decide(Confidence::new(0.99), true, false),
            ReviewAction::Accept
        );
    }

    #[test]
    fn validated_values_are_not_overwritten_without_policy() {
        let t = Thresholds::default();
        // High confidence, but replacing a validated value ⇒ review first.
        assert_eq!(
            t.decide(Confidence::new(0.99), false, true),
            ReviewAction::Review
        );
        assert_eq!(
            t.decide(Confidence::new(0.99), true, true),
            ReviewAction::Accept
        );
    }

    #[test]
    fn suggested_types_round_trip_through_the_schema_layer() {
        for (a, b, c) in [
            ("text", SuggestedType::Text, DeclaredType::Text),
            ("integer", SuggestedType::Int, DeclaredType::Int),
            ("boolean", SuggestedType::Bool, DeclaredType::Bool),
        ] {
            assert_eq!(SuggestedType::parse(a), Some(b));
            assert_eq!(b.to_declared(), c);
            assert_eq!(SuggestedType::from_declared(c), b);
        }
        assert_eq!(SuggestedType::parse("not-a-type"), None);
    }

    #[test]
    fn provenance_labels_are_explicit() {
        assert_eq!(Provenance::Deterministic.label(), "VALIDATED");
        assert_eq!(Provenance::AiSuggestion.label(), "AI SUGGESTION");
        assert!(Provenance::AiConfirmed.is_ai());
        assert!(!Provenance::Deterministic.is_ai());
    }
}
