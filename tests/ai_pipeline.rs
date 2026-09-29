// AI assistance pipeline — every test uses a local, deterministic provider.
// No network, no credentials, no external services: CI can run this offline.
//
// The contract under test is the one that matters operationally:
//   AI is optional, off by default, budgeted, and it can never destroy a
//   record that deterministic extraction already produced.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

use harvest::ai::schema::{
    AiOutcome, AiState, Band, Confidence, FieldSuggestion, Provenance, ProviderMetadata,
    ReviewAction, SuggestedType,
};
use harvest::ai::{
    apply_suggestions, candidate, normalization_for, redact, AiCfg, AiClient, AiError, AiField,
    AiProvider, AiRequest, AiResponse, AiTask, DeterministicProvider, DisabledProvider,
    FailingProvider, RawSuggestion, REDACTION_MARK,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

fn cfg_on() -> AiCfg {
    AiCfg {
        enabled: true,
        provider: "deterministic-local".to_string(),
        ..AiCfg::default()
    }
}

fn req(fields: Vec<(&str, &str)>) -> AiRequest {
    AiRequest {
        task: AiTask::Extract,
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

/// Counts calls so retry and budget behaviour is measured, not assumed.
struct Counting {
    calls: Arc<AtomicU32>,
    result: Result<AiResponse, AiError>,
}

impl Counting {
    fn new(counter: Arc<AtomicU32>, result: Result<AiResponse, AiError>) -> Self {
        Self {
            calls: counter,
            result,
        }
    }
}

impl AiProvider for Counting {
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

fn suggestion(field: &str, value: Value, confidence: f64) -> FieldSuggestion {
    FieldSuggestion {
        field: field.to_string(),
        value,
        suggested: SuggestedType::Text,
        confidence: Confidence::new(confidence),
        rationale: "test".to_string(),
        provenance: Provenance::AiSuggestion,
        replaces_validated: false,
    }
}

fn outcome_with(suggestions: Vec<FieldSuggestion>) -> AiOutcome {
    AiOutcome {
        state: AiState::Ok,
        suggestions,
        provider: Some(ProviderMetadata {
            name: "counting-test".to_string(),
            model: Some("counter-1".to_string()),
            ..ProviderMetadata::default()
        }),
        ..AiOutcome::default()
    }
}

#[test]
fn ai_is_disabled_by_default_and_says_so() {
    let mut c = AiClient::disabled();
    let out = c.assist(&req(vec![("title", "Mousetrap")]));
    assert_eq!(out.state, AiState::Disabled);
    assert!(
        !out.has_suggestions(),
        "a disabled provider must not suggest"
    );
    assert!(!c.status().enabled);
    assert_eq!(c.status().name, "disabled");
    assert_eq!(c.status().model, None);
    assert!(
        !c.status().external,
        "the disabled provider never leaves the process"
    );
}

#[test]
fn configured_but_not_enabled_stays_disabled() {
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
fn disabled_provider_answers_disabled_not_empty_success() {
    let mut c = AiClient::new(cfg_on(), Box::new(DisabledProvider));
    let out = c.assist(&req(vec![("a", "1")]));
    assert_eq!(out.state, AiState::Disabled);
    assert!(out.suggestions.is_empty());
}

#[test]
fn external_providers_require_explicit_consent() {
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
fn deterministic_provider_infers_types_from_value_shape() {
    let mut c = AiClient::new(cfg_on(), Box::new(DeterministicProvider));
    let out = c.assist(&req(vec![
        ("qty", "42"),
        ("price", "19.99"),
        ("in_stock", "true"),
        ("published", "2026-01-31"),
        ("payload", "{\"a\":1}"),
        ("tags", "a,b,c"),
        ("title", "A nicer mousetrap"),
    ]));
    assert_eq!(out.state, AiState::Ok);
    let get = |f: &str| {
        out.suggestions
            .iter()
            .find(|s| s.field == f)
            .expect("field")
            .clone()
    };
    assert_eq!(get("qty").suggested, SuggestedType::Int);
    assert_eq!(get("qty").value, json!(42));
    assert_eq!(get("qty").confidence.band(), Band::High);
    assert_eq!(get("price").suggested, SuggestedType::Number);
    assert_eq!(get("in_stock").suggested, SuggestedType::Bool);
    assert_eq!(get("in_stock").value, json!(true));
    assert_eq!(get("published").suggested, SuggestedType::Date);
    assert_eq!(
        get("published").confidence.band(),
        Band::Medium,
        "dates are review-grade"
    );
    assert_eq!(get("payload").suggested, SuggestedType::Json);
    assert_eq!(get("tags").suggested, SuggestedType::List);
    assert_eq!(get("title").suggested, SuggestedType::Text);
    assert!(out
        .suggestions
        .iter()
        .all(|s| s.provenance == Provenance::AiSuggestion));
    assert_eq!(
        out.provider.as_ref().expect("metadata").name,
        "deterministic-local"
    );
    assert!(!out
        .provider
        .as_ref()
        .unwrap()
        .model
        .as_ref()
        .unwrap()
        .is_empty());
}

#[test]
fn provider_metadata_cannot_carry_credentials() {
    let out = {
        let mut c = AiClient::new(cfg_on(), Box::new(DeterministicProvider));
        c.assist(&req(vec![("a", "1")]))
    };
    let rendered = serde_json::to_string(&out.provider).unwrap();
    for banned in [
        "api_key",
        "authorization",
        "secret",
        "prompt",
        "endpoint",
        "bearer",
    ] {
        assert!(
            !rendered.to_ascii_lowercase().contains(banned),
            "metadata leaked `{banned}`"
        );
    }
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
fn input_size_budget_is_checked_before_the_call() {
    let cfg = AiCfg {
        input_budget_bytes: 32,
        ..cfg_on()
    };
    let mut c = AiClient::new(cfg, Box::new(DeterministicProvider));
    let big = req(vec![("blob", &"x".repeat(4096))]);
    assert!(big.input_bytes() > 32);
    assert_eq!(c.assist(&big).state, AiState::BudgetExceeded);
    assert_eq!(c.status().requests_used, 0, "a refused call is not billed");
}

#[test]
fn provider_failure_is_reported_and_retries_are_bounded() {
    let counter = Arc::new(AtomicU32::new(0));
    let mut c = AiClient::new(
        AiCfg {
            max_retries: 3,
            ..cfg_on()
        },
        Box::new(Counting::new(
            counter.clone(),
            Err(AiError::Failed("boom".to_string())),
        )),
    );
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Failed);
    assert_eq!(counter.load(Ordering::SeqCst), 4, "1 attempt + 3 retries");
    assert_eq!(
        c.status().requests_used,
        0,
        "failed attempts are not billed"
    );
}

#[test]
fn slow_provider_trips_the_configured_timeout() {
    let mut c = AiClient::new(
        AiCfg {
            timeout_ms: 0,
            ..cfg_on()
        },
        Box::new(SlowProvider),
    );
    assert_eq!(c.assist(&req(vec![("a", "1")])).state, AiState::Timeout);
}

#[test]
fn malformed_provider_output_is_invalid_not_partially_applied() {
    let counter = Arc::new(AtomicU32::new(0));
    let bad = Ok(AiResponse {
        suggestions: vec![
            RawSuggestion {
                field: "price".to_string(),
                value: json!("12"),
                suggested: "quantum".to_string(), // not a type we know
                confidence: 0.9,
                rationale: "nonsense".to_string(),
            },
            RawSuggestion {
                field: "qty".to_string(),
                value: json!(3),
                suggested: "int".to_string(),
                confidence: 7.5, // outside [0,1]
                rationale: "overconfident".to_string(),
            },
        ],
        provider: ProviderMetadata::default(),
    });
    let mut c = AiClient::new(cfg_on(), Box::new(Counting::new(counter, bad)));
    let out = c.assist(&req(vec![("price", "12")]));
    assert_eq!(out.state, AiState::Invalid);
    assert!(
        out.suggestions.is_empty(),
        "no partial application of a bad payload"
    );
    assert_eq!(out.validations.len(), 2);
    assert!(out.validations.iter().all(|v| !v.ok));
}

#[test]
fn confidence_thresholds_route_suggestions() {
    let mut rec: Map<String, Value> = Map::new();
    let out = outcome_with(vec![
        suggestion("high", json!("a"), 0.99),
        suggestion("medium", json!("b"), 0.72),
        suggestion("low", json!("c"), 0.10),
    ]);
    let report = apply_suggestions(&mut rec, &out, &cfg_on());
    assert_eq!(report.accepted, 1);
    assert_eq!(report.review, 1);
    assert_eq!(report.rejected, 1);
    assert_eq!(rec.get("high"), Some(&json!("a")));
    assert!(
        rec.get("medium").is_none(),
        "medium confidence waits for review"
    );
    assert!(rec.get("low").is_none(), "low confidence is never written");
}

#[test]
fn ai_never_overwrites_a_validated_value_without_policy() {
    let mut rec: Map<String, Value> = Map::new();
    rec.insert("price".to_string(), json!("19.99"));
    let out = outcome_with(vec![suggestion("price", json!(19.99), 0.99)]);
    let report = apply_suggestions(&mut rec, &out, &cfg_on());
    assert_eq!(report.kept_validated, 1);
    assert_eq!(
        rec.get("price"),
        Some(&json!("19.99")),
        "deterministic value survives"
    );

    let mut permissive = cfg_on();
    permissive.overwrite_validated = true;
    let report = apply_suggestions(&mut rec, &out, &permissive);
    assert_eq!(report.accepted, 1);
    assert_eq!(rec.get("price"), Some(&json!(19.99)));
}

#[test]
fn ai_failure_leaves_deterministic_records_intact() {
    // The pipeline shape: extraction produced a record; AI then fails. The
    // record must come out of the stage unchanged.
    let mut record: Map<String, Value> = Map::new();
    record.insert("title".to_string(), json!("Mousetrap"));
    record.insert("price".to_string(), json!("19.99"));
    let before = record.clone();

    let mut c = AiClient::new(
        AiCfg {
            max_retries: 1,
            ..cfg_on()
        },
        Box::new(FailingProvider),
    );
    let out = c.assist(&req(vec![("title", "Mousetrap")]));
    assert_eq!(out.state, AiState::Failed);
    let report = apply_suggestions(&mut record, &out, &cfg_on());
    assert_eq!(report, Default::default(), "nothing was applied");
    assert_eq!(record, before, "deterministic data is untouched");
}

#[test]
fn candidates_fill_only_missing_fields() {
    let mut rec: Map<String, Value> = Map::new();
    rec.insert("sku".to_string(), json!("A-1"));
    let out = AiOutcome {
        state: AiState::Ok,
        candidates: vec![
            candidate("sku", json!("OTHER"), 0.99, "must not apply"),
            candidate("color", json!("red"), 0.99, "missing field"),
            candidate("size", json!("M"), 0.2, "too weak"),
        ],
        ..AiOutcome::default()
    };
    let report = apply_suggestions(&mut rec, &out, &cfg_on());
    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected, 1);
    assert_eq!(rec.get("sku"), Some(&json!("A-1")));
    assert_eq!(rec.get("color"), Some(&json!("red")));
    assert!(rec.get("size").is_none());
}

#[test]
fn sensitive_fields_are_redacted_before_a_request_is_built() {
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
        AiField {
            name: "customer_token".to_string(),
            value: "t-9".to_string(),
            deterministic: None,
            redacted: false,
        },
    ];
    let (out, report) = redact(&cfg, fields);
    assert_eq!(report.redacted, 3);
    let serialized = serde_json::to_string(&AiRequest {
        task: AiTask::Extract,
        fields: out,
        declared: Vec::new(),
    })
    .unwrap();
    assert!(!serialized.contains("sk_live_abc123"));
    assert!(!serialized.contains("Bearer xyz"));
    assert!(!serialized.contains("t-9"));
    assert!(serialized.contains(REDACTION_MARK));
}

#[test]
fn redacted_fields_receive_no_suggestions() {
    let cfg = cfg_on();
    let (fields, _) = redact(
        &cfg,
        vec![AiField {
            name: "session_token".to_string(),
            value: "abc".to_string(),
            deterministic: None,
            redacted: false,
        }],
    );
    let mut c = AiClient::new(cfg, Box::new(DeterministicProvider));
    let out = c.assist(&AiRequest {
        task: AiTask::Extract,
        fields,
        declared: Vec::new(),
    });
    assert!(
        out.suggestions.is_empty(),
        "no inference over redacted data"
    );
}

#[test]
fn normalization_suggestions_name_existing_transform_ops() {
    let n = normalization_for("price", " $1,299.00 ").expect("currency");
    assert_eq!(n.op, "parse_number");
    assert!(n.confidence.get() >= Confidence::MEDIUM);
    let t = normalization_for("title", "  padded  ").expect("trim");
    assert_eq!(t.op, "trim");
    let w = normalization_for("title", "a    b").expect("collapse");
    assert_eq!(w.op, "collapse_whitespace");
    assert!(normalization_for("title", "").is_none());
}

#[test]
fn review_actions_cover_the_documented_bands() {
    let t = harvest::ai::schema::Thresholds::default();
    let cases = [
        (0.99, ReviewAction::Accept),
        (0.75, ReviewAction::Review),
        (0.10, ReviewAction::Reject),
    ];
    for (confidence, expected) in cases {
        assert_eq!(t.decide(Confidence::new(confidence), true, false), expected);
    }
    // A high-confidence suggestion over a validated value still asks a human,
    // unless the configuration explicitly allows overwriting.
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
fn outcome_json_labels_provenance_for_the_console() {
    let out = outcome_with(vec![suggestion("title", json!("x"), 0.95)]);
    let json = out.to_json();
    assert_eq!(json["state"], "ok");
    assert_eq!(json["suggestions"][0]["label"], "AI SUGGESTION");
    assert_eq!(json["suggestions"][0]["provenance"], "ai-suggestion");
    assert_eq!(json["suggestions"][0]["band"], "high");
    let mut deterministic = suggestion("url", json!("https://x.test"), 1.0);
    deterministic.provenance = Provenance::Deterministic;
    assert_eq!(deterministic.provenance.label(), "VALIDATED");
}

#[test]
fn suggestions_are_declared_not_declared_types_round_trip() {
    // A suggestion can be checked against a declared schema because both speak
    // the schema layer's vocabulary.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for name in ["text", "int", "number", "bool", "date", "list", "json"] {
        let s = SuggestedType::parse(name).expect(name);
        seen.insert(s.to_declared().as_str());
        assert_eq!(
            SuggestedType::from_declared(s.to_declared()).as_str(),
            s.as_str()
        );
    }
    assert_eq!(seen.len(), 7);
    assert_eq!(SuggestedType::parse("not-a-type"), None);
}
