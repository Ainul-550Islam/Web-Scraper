//! Unit corpus for the AI assistance layer (`src/ai.rs` + `src/ai/schema.rs`).
//!
//! Every test uses local, deterministic providers — no network, no
//! credentials, no external services. The point is to prove the *contract*:
//! disabled stays disabled, budgets bite, malformed payloads are rejected,
//! and no suggestion is ever able to destroy deterministic data.

use super::*;
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread::sleep;
use std::time::Duration;

use crate::ai::schema::{AiState, Band, Provenance};

fn req(fields: Vec<(&str, &str)>) -> AiRequest {
    AiRequest {
        task: AiTask::Classify,
        fields: fields
            .into_iter()
            .map(|(name, value)| AiField {
                name: name.to_string(),
                value: value.to_string(),
                deterministic: None,
                redacted: false,
            })
            .collect(),
        declared: Vec::new(),
    }
}

fn cfg_on() -> AiCfg {
    AiCfg {
        enabled: true,
        provider: "deterministic-local".to_string(),
        ..AiCfg::default()
    }
}

/// Records how many times a provider was called, so retry behaviour is
/// measured rather than assumed.
struct CountingProvider {
    calls: AtomicU32,
    result: Result<AiResponse, AiError>,
}

impl CountingProvider {
    fn failing() -> Self {
        Self {
            calls: AtomicU32::new(0),
            result: Err(AiError::Failed("nope".to_string())),
        }
    }
    fn invalid() -> Self {
        Self {
            calls: AtomicU32::new(0),
            result: Ok(AiResponse {
                suggestions: vec![RawSuggestion {
                    field: "price".to_string(),
                    value: json!("x"),
                    suggested: "quantum".to_string(),
                    confidence: 0.9,
                    rationale: "nonsense type".to_string(),
                }],
                provider: ProviderMetadata::default(),
            }),
        }
    }
}

impl AiProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting-test"
    }
    fn model(&self) -> Option<&str> {
        Some("counter-1")
    }
    fn external(&self) -> bool {
        false
    }
    fn suggest(&self, _req: &AiRequest) -> Result<AiResponse, AiError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.result.clone()
    }
}

/// A provider that is slow enough to trip a zero-millisecond timeout: proves
/// the timeout path is measured, not decorative.
struct SlowProvider;

impl AiProvider for SlowProvider {
    fn name(&self) -> &str {
        "slow-test"
    }
    fn model(&self) -> Option<&str> {
        None
    }
    fn external(&self) -> bool {
        false
    }
    fn suggest(&self, _req: &AiRequest) -> Result<AiResponse, AiError> {
        sleep(Duration::from_millis(3));
        Ok(AiResponse::default())
    }
}

/// An "external" provider used to prove the privacy gate: without consent it
/// must never be called.
struct ExternalProvider;

impl AiProvider for ExternalProvider {
    fn name(&self) -> &str {
        "external-test"
    }
    fn model(&self) -> Option<&str> {
        Some("hosted-1")
    }
    fn external(&self) -> bool {
        true
    }
    fn suggest(&self, _req: &AiRequest) -> Result<AiResponse, AiError> {
        Ok(AiResponse::default())
    }
}

#[test]
fn default_is_disabled_and_says_so() {
    let mut c = AiClient::disabled();
    let out = c.assist(&req(vec![("a", "1")]));
    assert_eq!(out.state, AiState::Disabled);
    assert!(
        !out.has_suggestions(),
        "a disabled provider must never suggest"
    );
    assert!(!c.status().enabled);
    assert_eq!(c.status().name, "disabled");
}

#[test]
fn configured_but_off_stays_disabled() {
    let cfg = AiCfg {
        enabled: false,
        provider: "deterministic-local".to_string(),
        ..AiCfg::default()
    };
    assert_eq!(cfg.reason_disabled(), Some("ai.enabled is false"));
    let mut c = AiClient::new(cfg, Box::new(DeterministicProvider));
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Disabled);
}

#[test]
fn external_providers_require_consent() {
    let cfg = AiCfg {
        external_allowed: false,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(ExternalProvider));
    assert_eq!(
        c.assist(&req(vec![("a", "1")])).state,
        AiState::Unconfigured
    );
    assert!(!c.status().external_consent_ok);

    let cfg = AiCfg {
        external_allowed: true,
        ..cfg_on()
    };
    let c = AiClient::new(cfg, Box::new(ExternalProvider));
    assert!(c.status().external_consent_ok);
}

#[test]
fn deterministic_provider_classifies_by_shape() {
    let mut c = AiClient::new(cfg_on(), Box::new(DeterministicProvider));
    let out = c.assist(&req(vec![
        ("count", "42"),
        ("price", "19.99"),
        ("in_stock", "true"),
        ("published", "2026-01-31"),
        ("tags", "a,b,c"),
        ("title", "A nicer mousetrap"),
        ("empty", "   "),
    ]));
    assert_eq!(out.state, AiState::Ok);
    let by = |f: &str| {
        out.suggestions
            .iter()
            .find(|s| s.field == f)
            .expect("field")
            .clone()
    };
    assert_eq!(by("count").suggested, SuggestedType::Int);
    assert_eq!(by("count").value, json!(42));
    assert_eq!(by("count").confidence.band(), Band::High);
    assert_eq!(by("price").suggested, SuggestedType::Number);
    assert_eq!(by("price").confidence.band(), Band::High);
    assert_eq!(by("in_stock").suggested, SuggestedType::Bool);
    assert_eq!(by("in_stock").value, json!(true));
    assert_eq!(by("published").suggested, SuggestedType::Date);
    assert_eq!(by("published").confidence.band(), Band::Medium);
    assert_eq!(by("tags").suggested, SuggestedType::List);
    assert_eq!(by("title").suggested, SuggestedType::Text);
    assert_eq!(by("title").confidence.band(), Band::Medium);
    assert_eq!(by("empty").confidence.band(), Band::Low);
    assert!(out
        .suggestions
        .iter()
        .all(|s| s.provenance == Provenance::AiSuggestion));
}

#[test]
fn request_budget_is_consumed_and_enforced() {
    let cfg = AiCfg {
        request_budget: 2,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(DeterministicProvider));
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Ok);
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Ok);
    assert_eq!(c.remaining().0, 0);
    assert_eq!(
        c.assist(&req(vec![("a", "1")])).state,
        AiState::BudgetExceeded
    );
    assert_eq!(c.status().requests_used, 2);
}

#[test]
fn input_budget_is_enforced_before_the_call() {
    let cfg = AiCfg {
        input_budget_bytes: 10,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(DeterministicProvider));
    assert_eq!(
        c.assist(&req(vec![("a", "a much too large value")])).state,
        AiState::BudgetExceeded
    );
    assert_eq!(
        c.status().requests_used,
        0,
        "a refused call must not be billed"
    );
}

#[test]
fn provider_failure_is_reported_not_faked() {
    let cfg = AiCfg {
        max_retries: 2,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(FailingProvider));
    let out = c.assist(&req(vec![("a", "1")]));
    assert_eq!(out.state, AiState::Failed);
    assert!(!out.has_suggestions());
}

#[test]
fn retries_are_bounded_then_fail() {
    // One provider instance shared with the test so the attempt count is
    // measured, not assumed: max_retries=3 means four calls, then failure.
    let shared = std::sync::Arc::new(CountingProvider::failing());
    let mut c = AiClient::new(
        AiCfg {
            max_retries: 3,
            ..cfg_on()
        },
        Box::new(ArcProvider(shared.clone())),
    );
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Failed);
    assert_eq!(
        shared.calls.load(Ordering::SeqCst),
        4,
        "1 attempt + 3 retries"
    );
    assert_eq!(
        c.status().requests_used,
        0,
        "failed attempts are not billed"
    );
}

struct ArcProvider(std::sync::Arc<CountingProvider>);

impl AiProvider for ArcProvider {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn model(&self) -> Option<&str> {
        self.0.model()
    }
    fn external(&self) -> bool {
        self.0.external()
    }
    fn suggest(&self, req: &AiRequest) -> Result<AiResponse, AiError> {
        self.0.suggest(req)
    }
}

#[test]
fn malformed_payload_is_invalid_not_partially_applied() {
    let mut c = AiClient::new(cfg_on(), Box::new(CountingProvider::invalid()));
    let out = c.assist(&req(vec![("price", "12")]));
    assert_eq!(out.state, AiState::Invalid);
    assert!(out.suggestions.is_empty());
    assert_eq!(out.validations.len(), 1);
    assert!(!out.validations[0].ok);
}

#[test]
fn slow_provider_trips_timeout() {
    let cfg = AiCfg {
        timeout_ms: 0,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(SlowProvider));
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Timeout);
}

#[test]
fn sensitive_field_names_are_redacted_before_the_request_is_built() {
    let cfg = cfg_on();
    let fields = vec![
        AiField {
            name: "title".to_string(),
            value: "Mousetrap".to_string(),
            deterministic: None,
            redacted: false,
        },
        AiField {
            name: "api_key".to_string(),
            value: "sk_live_abc123".to_string(),
            deterministic: None,
            redacted: false,
        },
        AiField {
            name: "Authorization".to_string(),
            value: "Bearer xyz".to_string(),
            deterministic: None,
            redacted: false,
        },
    ];
    let (out, report) = redact(&cfg, fields);
    assert_eq!(report.redacted, 2);
    assert_eq!(out[0].value, "Mousetrap");
    assert_eq!(out[1].value, REDACTION_MARK);
    assert!(out[1].redacted);
    let serialized = serde_json::to_string(&AiRequest {
        task: AiTask::Extract,
        fields: out,
        declared: Vec::new(),
    })
    .unwrap();
    assert!(
        !serialized.contains("sk_live_abc123"),
        "secret must not reach the request"
    );
    assert!(!serialized.contains("Bearer xyz"));
}

#[test]
fn redaction_drops_the_value_from_suggestions() {
    let (fields, _) = redact(
        &cfg_on(),
        vec![AiField {
            name: "session_token".to_string(),
            value: "abc".to_string(),
            deterministic: None,
            redacted: false,
        }],
    );
    let mut c = AiClient::new(cfg_on(), Box::new(DeterministicProvider));
    let out = c.assist(&AiRequest {
        task: AiTask::Extract,
        fields,
        declared: Vec::new(),
    });
    assert!(
        out.suggestions.is_empty(),
        "no suggestion may be made on redacted data"
    );
}

#[test]
fn low_confidence_never_writes_to_the_record() {
    let mut rec = serde_json::Map::new();
    let outcome = AiOutcome {
        state: AiState::Ok,
        suggestions: vec![FieldSuggestion {
            field: "title".to_string(),
            value: json!("guessed"),
            suggested: SuggestedType::Text,
            confidence: Confidence::new(0.2),
            rationale: "weak".to_string(),
            provenance: Provenance::AiSuggestion,
            replaces_validated: false,
        }],
        ..AiOutcome::default()
    };
    let report = apply_suggestions(&mut rec, &outcome, &cfg_on());
    assert_eq!(report.rejected, 1);
    assert!(rec.get("title").is_none());
}

#[test]
fn high_confidence_does_not_overwrite_a_validated_value_by_default() {
    let mut rec = serde_json::Map::new();
    rec.insert("price".to_string(), json!("19.99"));
    let outcome = AiOutcome {
        state: AiState::Ok,
        suggestions: vec![FieldSuggestion {
            field: "price".to_string(),
            value: json!(19.99),
            suggested: SuggestedType::Number,
            confidence: Confidence::new(0.99),
            rationale: "shape".to_string(),
            provenance: Provenance::AiSuggestion,
            replaces_validated: true,
        }],
        ..AiOutcome::default()
    };
    let report = apply_suggestions(&mut rec, &outcome, &cfg_on());
    assert_eq!(report.kept_validated, 1);
    assert_eq!(
        rec.get("price"),
        Some(&json!("19.99")),
        "deterministic value survives"
    );

    let mut overwrite_cfg = cfg_on();
    overwrite_cfg.overwrite_validated = true;
    let report = apply_suggestions(&mut rec, &outcome, &overwrite_cfg);
    assert_eq!(report.accepted, 1);
    assert_eq!(rec.get("price"), Some(&json!(19.99)));
}

#[test]
fn candidates_fill_missing_fields_only() {
    let mut rec = serde_json::Map::new();
    rec.insert("sku".to_string(), json!("A-1"));
    let outcome = AiOutcome {
        state: AiState::Ok,
        candidates: vec![
            candidate("sku", json!("other"), 0.99, "should not apply"),
            candidate("color", json!("red"), 0.99, "missing field"),
        ],
        ..AiOutcome::default()
    };
    let report = apply_suggestions(&mut rec, &outcome, &cfg_on());
    assert_eq!(report.accepted, 1);
    assert_eq!(rec.get("sku"), Some(&json!("A-1")));
    assert_eq!(rec.get("color"), Some(&json!("red")));
}

#[test]
fn normalization_suggestions_name_real_transform_ops() {
    let n = normalization_for("price", " $1,299.00 ").expect("currency");
    assert_eq!(n.op, "parse_number");
    let t = normalization_for("title", "  padded  ").expect("trim");
    assert_eq!(t.op, "trim");
    let w = normalization_for("title", "a    b").expect("collapse");
    assert_eq!(w.op, "collapse_whitespace");
    assert!(normalization_for("title", "").is_none());
}

#[test]
fn provider_metadata_carries_no_credentials() {
    let meta = ProviderMetadata {
        name: "deterministic-local".to_string(),
        model: Some("shape-inference-1".to_string()),
        request_id: Some("det-1".to_string()),
        latency_ms: Some(0),
        input_bytes: Some(10),
        output_bytes: Some(20),
    };
    let json = meta.to_json();
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 6);
    for banned in ["key", "secret", "prompt", "endpoint", "token"] {
        assert!(
            !keys.iter().any(|k| k.contains(banned)),
            "{banned} must not be modelled"
        );
    }
}
