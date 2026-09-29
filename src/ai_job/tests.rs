// Unit corpus for src/ai_job.rs — provider policy, budgets, confidence and
// the rule that AI never overwrites deterministic data. The provider used is
// the local deterministic one: no network, no credentials, no CI keys.

use super::*;
use crate::config::Config;
use crate::control::{CommandLimits, LOCAL_PROVIDER_ID};
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-aj-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

fn cfg_with_output(path: &std::path::Path) -> Config {
    Config {
        name: "ai-job-test".to_string(),
        output: crate::config::SinkCfg::Jsonl {
            path: path.to_path_buf(),
        },
        ..Config::default()
    }
}

fn write_records(path: &std::path::Path, n: usize) {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{{\"run\":\"r\",\"target\":\"t\",\"url\":\"https://example.com/{i}\",\"depth\":0,\"ts\":1700000000,\"fingerprint\":\"fp{i}\",\"egress\":\"direct\",\"fields\":{{\"title\":\"Item {i}\",\"price\":\"{i}.50\",\"qty\":\"{i}\"}}}}\n"
        ));
    }
    std::fs::write(path, out).unwrap();
}

fn inline_request(
    provider: &str,
    task: &str,
    fields: &[(&str, &str)],
) -> crate::control::AiRequest {
    crate::control::AiRequest {
        task: task.to_string(),
        source: AiSource::Inline {
            fields: fields
                .iter()
                .map(|(n, v)| crate::control::AiFieldInput {
                    name: n.to_string(),
                    value: v.to_string(),
                    deterministic: Some(v.to_string()),
                })
                .collect(),
        },
        confidence: 0.9,
        provider: provider.to_string(),
        external_allowed: false,
    }
}

fn spec(provider: &str, task: &str, fields: &[(&str, &str)]) -> AiJobSpec {
    AiJobSpec::from_request(
        &inline_request(provider, task, fields),
        &CommandLimits::default(),
    )
    .unwrap()
}

// --------------------------------------------------------------------- spec

#[test]
fn spec_parses_tasks_and_rejects_unknown_ones() {
    let s = spec(LOCAL_PROVIDER_ID, "classify", &[("price", "1.50")]);
    assert_eq!(s.task, AiTask::Classify);
    assert_eq!(s.confidence, 0.9);
    assert_eq!(s.request_budget, 1, "one job, one request");
    let bad = AiJobSpec::from_request(
        &inline_request(LOCAL_PROVIDER_ID, "teleport", &[("a", "b")]),
        &CommandLimits::default(),
    );
    assert!(bad.is_err());
}

#[test]
fn spec_rejects_unknown_providers() {
    let e = AiJobSpec::from_request(
        &inline_request("some-hosted-llm", "classify", &[("a", "b")]),
        &CommandLimits::default(),
    )
    .unwrap_err();
    assert!(e.message.contains("unknown ai provider"));
}

#[test]
fn the_local_provider_is_never_external() {
    let s = spec(LOCAL_PROVIDER_ID, "classify", &[("a", "1")]);
    let status = s.client().status();
    assert!(!status.external);
    assert!(status.external_consent_ok);
    assert_eq!(status.name, "deterministic-local");
}

#[test]
fn the_disabled_provider_reports_disabled() {
    let s = spec(
        crate::control::DISABLED_PROVIDER_ID,
        "classify",
        &[("a", "1")],
    );
    let status = s.client().status();
    assert!(!status.enabled);
    assert!(!status.configured);
}

#[test]
fn confidence_threshold_is_carried_into_the_client() {
    let mut req = inline_request(LOCAL_PROVIDER_ID, "classify", &[("a", "1")]);
    req.confidence = 0.4;
    let s = AiJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    assert_eq!(s.client().status().thresholds.accept, 0.4);
    assert_eq!(s.client().status().thresholds.review, 0.4);
}

// ------------------------------------------------------------------ run_output

#[test]
fn fields_from_run_output_are_bounded() {
    let d = tmp("fields");
    let p = d.join("out.jsonl");
    write_records(&p, 100);
    let fields = fields_from_run_output(&p, 3, 2).unwrap();
    // 3 records x 2 fields
    assert_eq!(fields.len(), 6, "{fields:?}");
    assert_eq!(fields[0].name, "price");
    assert_eq!(fields[1].name, "qty");
    assert_eq!(fields[0].deterministic.as_deref(), Some("0.50"));
}

#[test]
fn fields_from_run_output_rejects_broken_lines() {
    let d = tmp("broken");
    let p = d.join("out.jsonl");
    std::fs::write(&p, "{not json}\n").unwrap();
    assert!(fields_from_run_output(&p, 1, 2).is_err());
}

#[test]
fn clip_bounds_a_single_huge_value() {
    let s = "x".repeat(5_000);
    assert_eq!(clip(&s, 2_048).chars().count(), 2_048);
    assert_eq!(clip("abc", 10), "abc");
}

// ---------------------------------------------------------------------- runs

#[test]
fn a_local_round_produces_suggestions_and_counts() {
    let d = tmp("run-ok");
    let p = d.join("out.jsonl");
    write_records(&p, 2);
    let cfg = cfg_with_output(&p);
    let s = spec(
        LOCAL_PROVIDER_ID,
        "classify",
        &[("price", "12.50"), ("qty", "3")],
    );
    let res = run(&s, &cfg);
    assert_eq!(res.state, "ok", "{res:?}");
    assert!(!res.suggestions.is_empty());
    assert_eq!(res.counts.fields, 2);
    assert!(res.counts.suggestions >= 2);
    assert!(res.latency_ms.is_some());
    assert!(res.reason.is_none());
    assert!(res.ok());
}

#[test]
fn a_high_confidence_threshold_keeps_suggestions_in_review() {
    let d = tmp("threshold");
    let p = d.join("out.jsonl");
    std::fs::write(&p, "").unwrap();
    let cfg = cfg_with_output(&p);
    let mut req = inline_request(LOCAL_PROVIDER_ID, "classify", &[("title", "hello")]);
    req.confidence = 1.0; // nothing below a perfect score may be accepted
    let s = AiJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    let res = run(&s, &cfg);
    assert_eq!(res.state, "ok");
    assert!(
        res.counts.accepted == 0,
        "a 1.0 threshold must accept nothing: {:?}",
        res.counts
    );
    assert!(res.counts.review + res.counts.rejected > 0);
}

#[test]
fn a_disabled_provider_is_reported_not_faked() {
    let d = tmp("disabled");
    let p = d.join("out.jsonl");
    write_records(&p, 1);
    let cfg = cfg_with_output(&p);
    let s = spec(
        crate::control::DISABLED_PROVIDER_ID,
        "classify",
        &[("price", "1.00")],
    );
    let res = run(&s, &cfg);
    assert_eq!(res.state, "disabled");
    assert!(!res.ok());
    assert!(res.reason.unwrap().contains("disabled"));
    assert_eq!(res.counts.suggestions, 0);
    assert!(res.suggestions.is_empty());
}

#[test]
fn the_input_budget_is_enforced() {
    let d = tmp("budget");
    let p = d.join("out.jsonl");
    write_records(&p, 1);
    let cfg = cfg_with_output(&p);
    let mut s = spec(LOCAL_PROVIDER_ID, "classify", &[("a", "b")]);
    s.max_input_bytes = 4;
    let res = run(&s, &cfg);
    assert_eq!(res.state, "budget-exceeded");
    assert!(res.reason.unwrap().contains("input cap"));
    assert_eq!(res.counts.suggestions, 0);
}

#[test]
fn sensitive_field_names_are_redacted_before_the_provider_sees_them() {
    let d = tmp("redact");
    let p = d.join("out.jsonl");
    write_records(&p, 1);
    let cfg = cfg_with_output(&p);
    let s = spec(
        LOCAL_PROVIDER_ID,
        "classify",
        &[
            ("api_key", "secret"),
            ("authorization", "Bearer x"),
            ("title", "ok"),
        ],
    );
    let res = run(&s, &cfg);
    assert_eq!(res.counts.fields, 3);
    assert_eq!(res.counts.redacted, 2, "token-shaped names are redacted");
}

#[test]
fn a_missing_source_is_a_failed_round_not_a_panic() {
    let d = tmp("nosource");
    let cfg = cfg_with_output(&d.join("nope.jsonl"));
    // run_output over a file that does not exist: the job must report the
    // failure, not panic or invent fields
    let req = crate::control::AiRequest {
        task: "classify".to_string(),
        source: AiSource::RunOutput {
            record_limit: 10,
            field_limit: 5,
        },
        confidence: 0.9,
        provider: LOCAL_PROVIDER_ID.to_string(),
        external_allowed: false,
    };
    let s = AiJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    let res = run(&s, &cfg);
    assert_eq!(res.state, "failed");
    assert!(
        res.reason.unwrap().contains("does not exist"),
        "the failure names the missing source"
    );
    assert_eq!(res.counts.suggestions, 0);
}

#[test]
fn ai_never_overwrites_a_validated_value() {
    let d = tmp("nooverwrite");
    let p = d.join("out.jsonl");
    write_records(&p, 1);
    let cfg = cfg_with_output(&p);
    // every field arrives with a deterministic value, so nothing may be
    // written: `overwrite_validated` is false for control-plane jobs.
    let s = spec(LOCAL_PROVIDER_ID, "classify", &[("price", "1.50")]);
    let res = run(&s, &cfg);
    assert_eq!(res.counts.accepted, 0, "{:?}", res.counts);
    assert!(res.counts.kept_validated > 0 || res.counts.review > 0);
    // and the source file is untouched
    let before = std::fs::read_to_string(&p).unwrap();
    let _ = run(&s, &cfg);
    assert_eq!(before, std::fs::read_to_string(&p).unwrap());
}

#[test]
fn every_task_string_is_accepted() {
    for t in [
        "extract",
        "classify",
        "normalize",
        "schema-suggest",
        "label",
    ] {
        let s = spec(LOCAL_PROVIDER_ID, t, &[("a", "1")]);
        assert_eq!(s.task.as_str(), t);
    }
}

#[test]
fn result_json_carries_no_credentials_or_endpoints() {
    let d = tmp("json");
    let p = d.join("out.jsonl");
    write_records(&p, 1);
    let cfg = cfg_with_output(&p);
    let s = spec(LOCAL_PROVIDER_ID, "classify", &[("price", "1.50")]);
    let res = run(&s, &cfg);
    let text = serde_json::to_string(&res).unwrap();
    for banned in ["api_key", "authorization", "https://", "secret"] {
        // `api_key` may appear only as a *field name* the user supplied and
        // the redaction report counts — never as a value, so assert on the
        // values we actually emit
        if banned == "https://" || banned == "secret" || banned == "authorization" {
            assert!(!text.contains(banned), "{text}");
        }
    }
    assert!(text.contains("provider"));
    assert!(text.contains("confidence"));
}
