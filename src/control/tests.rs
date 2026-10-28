// Unit corpus for src/control.rs — the command model, its bounds and its
// refusals. Everything here is deterministic and offline: no server, no
// network, no config file on disk.

use super::*;
use crate::config::{Config, TargetCfg};
use crate::control::CommandSpec as Spec;
use serde_json::json;

fn cfg_with(targets: &[&str]) -> Config {
    Config {
        targets: targets
            .iter()
            .map(|id| TargetCfg {
                id: id.to_string(),
                ..TargetCfg::default()
            })
            .collect(),
        ..Config::default()
    }
}

fn cmd_err(op: &str, body: serde_json::Value) -> ControlError {
    let mut v = body;
    v["op"] = json!(op);
    let raw = serde_json::to_vec(&v).unwrap();
    ControlCommand::from_request("admin-api", "req-1", &raw, None).unwrap_err()
}

fn cmd(op: &str, body: serde_json::Value) -> ControlCommand {
    let mut v = body;
    v["op"] = json!(op);
    let raw = serde_json::to_vec(&v).unwrap();
    ControlCommand::from_request("admin-api", "req-1", &raw, None).unwrap()
}

// ------------------------------------------------------------------ identity

#[test]
fn command_ids_are_unique_and_displayable() {
    let a = CommandId::next();
    let b = CommandId::next();
    assert_ne!(a, b);
    assert!(a.as_str().starts_with("cmd-"));
    assert_eq!(a.to_string(), a.0);
}

#[test]
fn job_id_parse_rejects_traversal_shaped_input() {
    assert!(ControlJobId::parse("job-1").is_some());
    assert!(ControlJobId::parse("job-").is_none());
    assert!(ControlJobId::parse("job-abc").is_none());
    assert!(ControlJobId::parse("job-1/../../etc").is_none());
    assert!(ControlJobId::parse("../../etc/passwd").is_none());
    assert!(ControlJobId::parse("job-999999999999999999999").is_none());
}

#[test]
fn artifact_id_parse_is_equally_strict() {
    assert!(ArtifactId::parse("art-12").is_some());
    assert!(ArtifactId::parse("art--12").is_none());
    assert!(ArtifactId::parse("art-1;rm").is_none());
    assert!(ArtifactId::parse("").is_none());
}

// ---------------------------------------------------------------- operations

#[test]
fn operation_strings_round_trip() {
    for (s, op) in [
        ("crawl", Operation::Crawl),
        ("export", Operation::Export),
        ("ai", Operation::AiAssist),
        ("verify", Operation::Verify),
        ("artifact", Operation::ArtifactInspect),
    ] {
        assert_eq!(Operation::parse(s), Some(op));
        assert_eq!(op.as_str(), s);
    }
    assert_eq!(Operation::parse("nope"), None);
    assert!(Operation::Crawl.creates_job());
    assert!(!Operation::Verify.creates_job());
}

// -------------------------------------------------------------------- limits

#[test]
fn limits_default_are_within_every_ceiling() {
    CommandLimits::default().validate().unwrap();
}

#[test]
fn limits_reject_every_over_ceiling_value() {
    let base = CommandLimits::default();
    let mut l = base;
    l.max_pages = MAX_PAGES_CEILING + 1;
    assert_eq!(l.validate().unwrap_err().status(), 400);

    let mut l = base;
    l.max_depth = MAX_DEPTH_CEILING + 1;
    assert!(l.validate().is_err());

    let mut l = base;
    l.max_records = MAX_RECORDS_CEILING + 1;
    assert!(l.validate().is_err());

    let mut l = base;
    l.max_export_bytes = MAX_EXPORT_BYTES_CEILING + 1;
    assert!(l.validate().is_err());

    let mut l = base;
    l.max_ai_input_bytes = MAX_AI_INPUT_CEILING + 1;
    assert!(l.validate().is_err());

    let mut l = base;
    l.timeout_secs = MAX_TIMEOUT_SECS + 1;
    assert!(l.validate().is_err());
}

#[test]
fn limits_reject_zero_budgets() {
    let base = CommandLimits::default();
    for l in [
        CommandLimits {
            max_pages: 0,
            ..base
        },
        CommandLimits {
            max_records: 0,
            ..base
        },
        CommandLimits {
            max_export_bytes: 0,
            ..base
        },
        CommandLimits {
            max_ai_input_bytes: 0,
            ..base
        },
        CommandLimits {
            timeout_secs: 0,
            ..base
        },
    ] {
        assert!(l.validate().is_err(), "{l:?} should be refused");
    }
    // depth 0 is legitimate: start URLs only.
    CommandLimits {
        max_depth: 0,
        ..base
    }
    .validate()
    .unwrap();
}

#[test]
fn limits_apply_overrides_and_reject_unknown_keys() {
    let mut l = CommandLimits::default();
    l.apply_overrides(&json!({ "max_pages": 7, "max_depth": 2 }))
        .unwrap();
    assert_eq!(l.max_pages, 7);
    assert_eq!(l.max_depth, 2);
    let e = l.apply_overrides(&json!({ "nope": 1 })).unwrap_err();
    assert!(e.message.contains("unknown limit"));
    let e = l.apply_overrides(&json!({ "max_pages": "7" })).unwrap_err();
    assert!(e.message.contains("must be a non-negative integer"));
}

#[test]
fn limits_clamp_down_to_the_attached_config() {
    let mut cfg = cfg_with(&["a"]);
    cfg.max_pages = 5;
    cfg.max_records_per_run = 42;
    cfg.targets[0].max_depth = 1;
    let mut l = CommandLimits {
        max_pages: 10_000,
        max_depth: 9,
        max_records: 1_000_000,
        ..CommandLimits::default()
    };
    l.clamp_to_config(&cfg);
    assert_eq!(l.max_pages, 5, "a command may never widen the config");
    assert_eq!(l.max_depth, 1);
    assert_eq!(l.max_records, 42);
}

#[test]
fn limits_clamp_ignores_unlimited_config_values() {
    let mut cfg = cfg_with(&["a"]);
    cfg.max_pages = 0;
    cfg.max_records_per_run = 0;
    let mut l = CommandLimits::default();
    l.clamp_to_config(&cfg);
    assert_eq!(l.max_pages, CommandLimits::default().max_pages);
}

// ------------------------------------------------------------------ commands

#[test]
fn crawl_command_parses_and_requires_known_targets() {
    let c = cmd("crawl", json!({ "targets": ["a"] }));
    assert_eq!(c.operation(), Operation::Crawl);
    assert!(matches!(c.spec, Spec::Crawl(ref r) if r.targets == vec!["a".to_string()]));
    assert!(ControlCommand::validate(&c, Some(&cfg_with(&["a"]))).is_ok());

    let bad = cmd("crawl", json!({ "targets": ["zzz"] }));
    let e = ControlCommand::validate(&bad, Some(&cfg_with(&["a"]))).unwrap_err();
    assert_eq!(e.status(), 400);
    assert!(e.message.contains("unknown target `zzz`"));
}

#[test]
fn crawl_command_without_config_is_unavailable() {
    let c = cmd("crawl", json!({ "targets": ["a"] }));
    let e = ControlCommand::validate(&c, None).unwrap_err();
    assert_eq!(e.status(), 503);
}

#[test]
fn ambiguous_target_selection_is_refused() {
    let c = cmd("crawl", json!({}));
    let e = ControlCommand::validate(&c, Some(&cfg_with(&["a", "b"]))).unwrap_err();
    assert!(e.message.contains("declares 2 targets"));
    // a single-target config is unambiguous, so an empty selection is fine
    assert!(ControlCommand::validate(&c, Some(&cfg_with(&["a"]))).is_ok());
}

#[test]
fn export_command_validates_formats_and_bundle() {
    let c = cmd(
        "export",
        json!({ "formats": ["csv", "jsonl"], "bundle": true }),
    );
    match &c.spec {
        Spec::Export(e) => {
            assert!(e.bundle);
            assert_eq!(e.formats, vec!["csv", "jsonl"]);
        }
        other => panic!("wrong spec: {other:?}"),
    }
    let bad = cmd("export", json!({ "formats": ["csv", "docx"] }));
    let e = bad.validate(None).unwrap_err();
    assert!(e.message.contains("unsupported export format `docx`"));
    assert!(
        e.message.contains("parquet"),
        "the reply lists what is supported"
    );

    let empty = cmd("export", json!({ "formats": [] }));
    assert!(empty
        .validate(None)
        .unwrap_err()
        .message
        .contains("at least one"));

    let zip_only = cmd("export", json!({ "formats": ["zip"], "bundle": true }));
    assert!(zip_only
        .validate(None)
        .unwrap_err()
        .message
        .contains("payload format"));
}

#[test]
fn export_command_needs_a_configured_source() {
    // no config attached at all
    let c = cmd("export", json!({ "formats": ["csv"] }));
    assert_eq!(c.validate(None).unwrap_err().status(), 503);
    // config attached, but the sink is not a file
    let cfg = crate::config::Config {
        output: crate::config::SinkCfg::Null {},
        ..cfg_with(&["a"])
    };
    let e = c.validate(Some(&cfg)).unwrap_err();
    assert_eq!(e.status(), 503);
    assert!(e.message.contains("not a file"));
}

#[test]
fn ai_command_validates_task_confidence_and_provider() {
    let c = cmd(
        "ai",
        json!({ "task": "classify", "provider": "deterministic-local",
                "source": { "kind": "inline", "fields": [{"name": "price", "value": "$1.00"}] } }),
    );
    assert_eq!(c.operation(), Operation::AiAssist);
    assert!(c.validate(None).is_ok());

    let bad_task = cmd("ai", json!({ "task": "teleport" }));
    assert!(bad_task
        .validate(None)
        .unwrap_err()
        .message
        .contains("unsupported ai task"));

    // an out-of-range threshold is refused at parse time: the command never
    // exists, so no job can be created from it
    let e = cmd_err("ai", json!({ "task": "classify", "confidence": 4.0 }));
    assert!(e.message.contains("[0, 1]"));
    assert_eq!(e.status(), 400);
    assert!(cmd_err("ai", json!({ "task": "classify", "confidence": -0.1 })).status() == 400);
    let ok = cmd("ai", json!({ "task": "classify", "confidence": 0.65 }));
    assert_eq!(
        ok.validate(None).unwrap_err().status(),
        503,
        "no source configured yet"
    );

    let bad_provider = cmd("ai", json!({ "task": "classify", "provider": "openai" }));
    let e = bad_provider.validate(None).unwrap_err();
    assert!(e.message.contains("unknown ai provider"));
    assert!(e.message.contains("deterministic-local"));
}

#[test]
fn ai_external_consent_cannot_summon_a_provider() {
    let c = cmd(
        "ai",
        json!({ "task": "classify", "provider": "deterministic-local", "external_allowed": true,
                "source": { "kind": "inline", "fields": [{"name": "a", "value": "b"}] } }),
    );
    assert!(c.validate(None).is_ok());
    let d = cmd(
        "ai",
        json!({ "task": "classify", "provider": "disabled", "external_allowed": true }),
    );
    assert!(d.validate(None).unwrap_err().message.contains("no effect"));
}

#[test]
fn ai_inline_fields_are_bounded() {
    let big = "x".repeat(MAX_AI_FIELD_VALUE_BYTES + 1);
    let raw = json!({
        "op": "ai", "task": "classify",
        "source": { "kind": "inline", "fields": [{"name": "a", "value": big}] }
    });
    let bytes = serde_json::to_vec(&raw).unwrap();
    let e = ControlCommand::from_request("a", "c", &bytes, None).unwrap_err();
    assert!(e.message.contains("exceeds"));

    let many: Vec<serde_json::Value> = (0..MAX_AI_FIELDS + 1)
        .map(|i| json!({ "name": format!("f{i}"), "value": "v" }))
        .collect();
    let raw =
        json!({ "op": "ai", "task": "classify", "source": { "kind": "inline", "fields": many } });
    let bytes = serde_json::to_vec(&raw).unwrap();
    // the per-field check trips first: a single oversized value is a 400
    let e = ControlCommand::from_request("a", "c", &bytes, None).unwrap_err();
    assert_eq!(e.status(), 400);
    assert!(e.message.contains("exceeds"));
}

#[test]
fn ai_run_output_limits_must_be_positive() {
    for source in [
        json!({"kind":"run_output","record_limit":0}),
        json!({"kind":"run_output","field_limit":0}),
    ] {
        let raw = json!({"op":"ai","task":"classify","source":source});
        let bytes = serde_json::to_vec(&raw).unwrap();
        let error = ControlCommand::from_request("a", "c", &bytes, None).unwrap_err();
        assert_eq!(error.status(), 400);
        assert!(error.message.contains("at least 1"));
    }
}

#[test]
fn ai_run_output_without_config_is_unavailable() {
    let c = cmd(
        "ai",
        json!({ "task": "classify", "source": { "kind": "run_output" } }),
    );
    let e = c.validate(None).unwrap_err();
    assert_eq!(e.status(), 503);
    // a config whose declared sink file does not exist is refused too: the
    // source discipline is the same as an export's
    let d = std::env::temp_dir().join(format!("harvest-ctl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let cfg = crate::config::Config {
        output: crate::config::SinkCfg::Jsonl {
            path: d.join("never-written.jsonl"),
        },
        ..cfg_with(&["a"])
    };
    assert!(
        c.validate(Some(&cfg)).is_err(),
        "source file missing => refused"
    );
}

#[test]
fn verify_and_inspect_validate_their_ids() {
    let c = cmd("verify", json!({ "job": "job-1" }));
    assert!(c.validate(None).is_ok());
    let both = cmd("verify", json!({ "job": "job-1", "artifact": "art-1" }));
    assert!(both
        .validate(None)
        .unwrap_err()
        .message
        .contains("exactly one"));
    // "neither field" is refused while parsing, before a command exists
    assert!(cmd_err("verify", json!({})).message.contains("needs `job`"));
    let bad = cmd("verify", json!({ "job": "../../etc" }));
    assert!(bad
        .validate(None)
        .unwrap_err()
        .message
        .contains("not a control job id"));

    let a = cmd("artifact", json!({ "artifact": "art-3" }));
    assert!(a.validate(None).is_ok());
    let bad_a = cmd("artifact", json!({ "artifact": "art-3/../x" }));
    assert!(bad_a.validate(None).unwrap_err().status() == 400);
}

#[test]
fn malformed_bodies_are_refused_not_coerced() {
    assert!(ControlCommand::from_request("a", "c", b"", None)
        .unwrap_err()
        .message
        .contains("needs a JSON body"));
    let e = ControlCommand::from_request("a", "c", b"{not json", None).unwrap_err();
    assert!(e.message.contains("not valid JSON"));
    assert_eq!(e.status(), 400);
    let e = ControlCommand::from_request("a", "c", b"[1,2]", None).unwrap_err();
    assert!(e.message.contains("must be a JSON object"));
    let e = ControlCommand::from_request("a", "c", b"{\"op\":\"mine\"}", None).unwrap_err();
    assert!(e.message.contains("unsupported op"));
    let e = ControlCommand::from_request("a", "c", b"{}", None).unwrap_err();
    assert!(e.message.contains("missing `op`"));
}

#[test]
fn oversized_bodies_are_refused_with_413() {
    let huge = vec![b'x'; COMMAND_BODY_LIMIT as usize + 1];
    let e = ControlCommand::from_request("a", "c", &huge, None).unwrap_err();
    assert_eq!(e.status(), 413);
}

#[test]
fn body_hash_is_stable_and_command_records_actor_and_time() {
    let raw = br#"{"op":"verify","job":"job-1"}"#;
    let a = ControlCommand::from_request("admin-api", "req-9", raw, None).unwrap();
    let b = ControlCommand::from_request("admin-api", "req-9", raw, None).unwrap();
    assert_eq!(a.body_hash, b.body_hash);
    assert_ne!(a.id, b.id, "identity is never reused");
    assert_eq!(a.actor, "admin-api");
    assert_eq!(a.corr, "req-9");
    assert!(a.created_ts > 0);
    assert_eq!(a.audit_target(), Some("job-1".to_string()));
}

#[test]
fn audit_targets_never_carry_payloads() {
    let c = cmd("crawl", json!({ "targets": ["a", "b"] }));
    assert_eq!(c.audit_target(), Some("a,b".to_string()));
    let c = cmd("export", json!({ "formats": ["csv"] }));
    assert_eq!(c.audit_target(), Some("csv".to_string()));
}

// --------------------------------------------------------------- idempotency

#[test]
fn idempotency_replays_identical_requests() {
    let idx = IdempotencyIndex::new();
    assert!(matches!(
        idx.check("admin-api", Operation::Crawl, "k1", "hash-a"),
        Idempotency::Fresh
    ));
    idx.bind("admin-api", Operation::Crawl, "k1", "hash-a", "job-1");
    match idx.check("admin-api", Operation::Crawl, "k1", "hash-a") {
        Idempotency::Replay(j) => assert_eq!(j, "job-1"),
        other => panic!("expected a replay, got {other:?}"),
    }
}

#[test]
fn idempotency_conflicts_on_reuse_with_a_different_body() {
    let idx = IdempotencyIndex::new();
    idx.bind("admin-api", Operation::Crawl, "k1", "hash-a", "job-1");
    assert!(matches!(
        idx.check("admin-api", Operation::Crawl, "k1", "hash-b"),
        Idempotency::Conflict
    ));
    assert!(matches!(
        idx.check("someone-else", Operation::Crawl, "k1", "hash-a"),
        Idempotency::Conflict
    ));
    assert!(matches!(
        idx.check("admin-api", Operation::Export, "k1", "hash-a"),
        Idempotency::Conflict
    ));
}

#[test]
fn idempotency_index_is_bounded() {
    let idx = IdempotencyIndex::new();
    for i in 0..IDEMPOTENCY_CAP + 20 {
        idx.bind(
            "a",
            Operation::Crawl,
            &format!("k{i}"),
            "h",
            &format!("job-{i}"),
        );
    }
    assert_eq!(idx.len(), IDEMPOTENCY_CAP);
    assert!(!idx.is_empty());
}

#[test]
fn idempotency_keys_are_validated() {
    assert_eq!(validate_idempotency_key(None).unwrap(), None);
    assert_eq!(
        validate_idempotency_key(Some("retry-1")).unwrap(),
        Some("retry-1".to_string())
    );
    assert!(validate_idempotency_key(Some("bad key")).is_err());
    assert!(validate_idempotency_key(Some("")).is_err());
    assert!(validate_idempotency_key(Some(&"x".repeat(65))).is_err());
}

// ------------------------------------------------------------------- helpers

#[test]
fn safe_ids_are_narrow() {
    assert!(is_safe_id("target-1"));
    assert!(is_safe_id("Target_2"));
    assert!(!is_safe_id(""));
    assert!(!is_safe_id("a.b"));
    assert!(!is_safe_id("a/b"));
    assert!(!is_safe_id("a b"));
    assert!(!is_safe_id(&"x".repeat(65)));
}

#[test]
fn sha256_hex_matches_the_known_digest() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn control_errors_map_to_stable_statuses() {
    assert_eq!(ControlError::unauthorized().to_api().status, 401);
    assert_eq!(ControlError::forbidden("nope").to_api().status, 403);
    assert_eq!(ControlError::invalid("nope").to_api().status, 400);
    assert_eq!(ControlError::not_found("nope").to_api().status, 404);
    assert_eq!(ControlError::conflict("nope").to_api().status, 409);
    assert_eq!(ControlError::unavailable("nope").to_api().status, 503);
    let e = ControlError::too_large(64);
    assert_eq!(e.status(), 413);
    assert!(e.to_api().message.contains("64"));
    // the 401 body is the same constant the rest of the admin API returns
    assert_eq!(
        ControlError::unauthorized().message,
        crate::admin::UNAUTHORIZED_MESSAGE
    );
}

#[test]
fn phase2_emit_restart_identity() {
    if std::env::var("HARVEST_PHASE2_EMIT_ID").as_deref() == Ok("1") {
        println!("RESTART-ID:{}:{}", ControlJobId::next(), ArtifactId::next());
    }
}

#[test]
fn phase2_fresh_processes_cannot_reissue_old_job_or_artifact_ids() {
    let child = || {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "control::tests::phase2_emit_restart_identity",
                "--nocapture",
            ])
            .env("HARVEST_PHASE2_EMIT_ID", "1")
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .find(|line| line.starts_with("RESTART-ID:"))
            .unwrap()
            .to_string()
    };
    let first = child();
    let second = child();
    assert_ne!(
        first, second,
        "server restart must not alias immutable directories or public artifact links"
    );
    let id = ControlJobId::next();
    assert!(ControlJobId::parse(id.as_str()).is_some());
    let id = ArtifactId::next();
    assert!(ArtifactId::parse(id.as_str()).is_some());
    assert!(ControlJobId::parse("job-../secret-1").is_none());
    assert!(ArtifactId::parse("art-123-1").is_none());
}
