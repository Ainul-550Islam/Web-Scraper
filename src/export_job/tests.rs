// Unit corpus for src/export_job.rs — the export job contract. These run the
// real exporter over a real JSONL file, so the assertions are about bytes
// that exist, not about a mock.

use super::*;
use crate::config::Config;
use crate::control::CommandLimits;
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-ej-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

/// A config whose sink is the given JSONL file.
fn cfg_with_output(path: &std::path::Path) -> Config {
    Config {
        name: "export-job-test".to_string(),
        output: SinkCfg::Jsonl {
            path: path.to_path_buf(),
        },
        schedule: crate::config::ScheduleCfg {
            history_path: path.parent().unwrap().join("runs.jsonl"),
            ..crate::config::ScheduleCfg::default()
        },
        ..Config::default()
    }
}

fn records(path: &std::path::Path, n: usize) {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{{\"run\":\"r\",\"target\":\"t\",\"url\":\"https://example.com/{i}\",\"depth\":0,\"ts\":1700000000,\"fingerprint\":\"fp{i}\",\"egress\":\"direct\",\"fields\":{{\"title\":\"T{i}\",\"price\":\"{}\"}}}}\n",
            i
        ));
    }
    std::fs::write(path, out).unwrap();
}

fn spec(formats: &[&str], verify: bool) -> ExportJobSpec {
    let req = ExportRequest {
        formats: formats.iter().map(|s| s.to_string()).collect(),
        dataset: Some("unit".to_string()),
        bundle: formats.contains(&"zip"),
        verify,
        max_records: None,
    };
    ExportJobSpec::from_request(&req, &CommandLimits::default()).unwrap()
}

// ------------------------------------------------------------------- sources

#[test]
fn source_files_uses_the_configured_sink() {
    let d = tmp("src");
    let p = d.join("out.jsonl");
    records(&p, 3);
    let cfg = cfg_with_output(&p);
    assert_eq!(source_files(&cfg).unwrap(), vec![p]);
}

#[test]
fn source_files_refuses_when_nothing_has_been_written() {
    let d = tmp("missing");
    let cfg = cfg_with_output(&d.join("nope.jsonl"));
    let e = source_files(&cfg).unwrap_err();
    assert_eq!(e.status(), 503);
    assert!(e.message.contains("does not exist yet"));
}

#[test]
fn source_files_refuses_non_file_sinks() {
    let cfg = Config {
        output: SinkCfg::Null {},
        ..Config::default()
    };
    assert!(source_files(&cfg)
        .unwrap_err()
        .message
        .contains("not a file"));
    let cfg = Config {
        output: SinkCfg::Http {
            url: "http://127.0.0.1:1/x".to_string(),
            batch_max: 1,
            timeout_secs: 1,
            auth: None,
        },
        ..Config::default()
    };
    assert!(source_files(&cfg).is_err());
}

#[test]
fn source_files_reads_a_partitioned_dataset_dir() {
    let d = tmp("dataset");
    let dir = d.join("ds");
    std::fs::create_dir_all(&dir).unwrap();
    records(&dir.join("part-a.jsonl"), 2);
    records(&dir.join("part-b.jsonl"), 2);
    std::fs::write(dir.join("ignored.txt"), "x").unwrap();
    let cfg = Config {
        output: SinkCfg::Dataset {
            dir: dir.clone(),
            partition: "none".to_string(),
        },
        ..Config::default()
    };
    let files = source_files(&cfg).unwrap();
    assert_eq!(files.len(), 2, "only .jsonl parts are sources: {files:?}");
}

// ---------------------------------------------------------------------- spec

#[test]
fn spec_dedupes_formats_and_adds_zip_when_bundling() {
    let req = ExportRequest {
        formats: vec!["csv".into(), "csv".into(), "jsonl".into()],
        dataset: None,
        bundle: true,
        verify: false,
        max_records: None,
    };
    let s = ExportJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    assert_eq!(s.formats.len(), 3);
    assert!(s.bundle);
    assert!(s.formats.contains(&ExportFormat::Zip));
    assert_eq!(s.format_names(), vec!["csv", "jsonl", "zip"]);
}

#[test]
fn spec_respects_the_command_limit() {
    let req = ExportRequest {
        formats: vec!["csv".into()],
        dataset: None,
        bundle: false,
        verify: false,
        max_records: Some(10_000_000),
    };
    let s = ExportJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    assert_eq!(s.max_records, CommandLimits::default().max_records);
    let req = ExportRequest {
        max_records: Some(5),
        ..req
    };
    let s = ExportJobSpec::from_request(&req, &CommandLimits::default()).unwrap();
    assert_eq!(s.max_records, 5);
}

#[test]
fn spec_refuses_unknown_formats_and_zip_only_bundles() {
    let req = ExportRequest {
        formats: vec!["docx".into()],
        dataset: None,
        bundle: false,
        verify: false,
        max_records: None,
    };
    assert!(ExportJobSpec::from_request(&req, &CommandLimits::default()).is_err());
    let req = ExportRequest {
        formats: vec!["zip".into()],
        bundle: true,
        ..req
    };
    assert!(ExportJobSpec::from_request(&req, &CommandLimits::default()).is_err());
}

// ---------------------------------------------------------------- truncation

#[test]
fn truncate_inputs_stops_at_the_record_cap() {
    let d = tmp("trunc");
    let p = d.join("out.jsonl");
    records(&p, 50);
    let work = d.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let staged = truncate_inputs(&[p], &work, 7).unwrap();
    let body = std::fs::read_to_string(&staged[0]).unwrap();
    assert_eq!(body.lines().count(), 7, "exactly the cap, no more");
    assert!(body.contains("\"url\":\"https://example.com/0\""));
    assert!(!body.contains("example.com/8"));
}

#[test]
fn truncate_inputs_skips_blank_lines_and_reports_empty_sources() {
    let d = tmp("blank");
    let p = d.join("out.jsonl");
    std::fs::write(&p, "\n\n\n").unwrap();
    let work = d.join("work");
    std::fs::create_dir_all(&work).unwrap();
    assert!(truncate_inputs(&[p], &work, 10).is_err());
}

// ----------------------------------------------------------------- real runs

#[test]
fn run_writes_the_canonical_pair_and_a_manifest() {
    let d = tmp("pair");
    let p = d.join("out.jsonl");
    records(&p, 12);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let (result, files) = run(&spec(&["csv", "jsonl"], false), &cfg, &out).unwrap();

    assert_eq!(result.records, 12);
    assert_eq!(result.formats, vec!["csv", "jsonl"]);
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"records.csv"), "{names:?}");
    assert!(names.contains(&"records.jsonl"), "{names:?}");
    assert!(names.contains(&"manifest.json"), "{names:?}");
    assert!(names.contains(&"checksums.txt"), "{names:?}");
    assert!(result.total_bytes > 0);
    assert_eq!(result.verification.as_str(), "not_requested");
    // every recorded hash is the real hash of the file on disk
    for f in &files {
        let body = std::fs::read(&f.path).unwrap();
        assert_eq!(f.sha256, crate::control::sha256_hex(&body));
        assert_eq!(f.bytes, body.len() as u64);
    }
}

#[test]
fn run_verifies_when_asked_and_reports_the_basis() {
    let d = tmp("verify");
    let p = d.join("out.jsonl");
    records(&p, 5);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let (result, _) = run(&spec(&["csv", "jsonl"], true), &cfg, &out).unwrap();
    match &result.verification {
        VerificationState::Ok {
            records,
            records_basis,
            files,
        } => {
            assert_eq!(*records, 5);
            assert_eq!(records_basis, "jsonl-lines");
            assert!(*files >= 2);
        }
        other => panic!("expected a passing verification, got {other:?}"),
    }
}

#[test]
fn a_tampered_delivery_fails_its_verification() {
    let d = tmp("tamper");
    let p = d.join("out.jsonl");
    records(&p, 5);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let (_, files) = run(&spec(&["csv", "jsonl"], false), &cfg, &out).unwrap();
    // flip one byte in a payload after the fact
    let victim = files
        .iter()
        .find(|f| f.name == "records.csv")
        .expect("csv exists");
    let mut body = std::fs::read(&victim.path).unwrap();
    let last = body.len() - 1;
    body[last] = body[last].wrapping_add(1);
    std::fs::write(&victim.path, &body).unwrap();

    match verify_dir(&out) {
        VerificationState::Failed { reason } => {
            assert!(reason.contains("manifest check"), "{reason}");
        }
        other => panic!("a tampered payload must fail: {other:?}"),
    }
}

#[test]
fn a_deleted_payload_fails_its_verification() {
    let d = tmp("deleted");
    let p = d.join("out.jsonl");
    records(&p, 4);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let (_, files) = run(&spec(&["jsonl"], false), &cfg, &out).unwrap();
    std::fs::remove_file(
        &files
            .iter()
            .find(|f| f.name == "records.jsonl")
            .unwrap()
            .path,
    )
    .unwrap();
    assert!(matches!(verify_dir(&out), VerificationState::Failed { .. }));
}

#[test]
fn verify_of_a_directory_without_a_manifest_fails() {
    let d = tmp("nomanifest");
    match verify_dir(&d) {
        VerificationState::Failed { reason } => assert!(reason.contains("no manifest.json")),
        other => panic!("{other:?}"),
    }
}

#[test]
fn all_seven_formats_render_through_the_same_pipeline() {
    let d = tmp("all");
    let p = d.join("out.jsonl");
    records(&p, 9);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let (result, files) = run(
        &spec(
            &["csv", "jsonl", "xlsx", "pdf", "parquet", "xml", "zip"],
            true,
        ),
        &cfg,
        &out,
    )
    .unwrap();
    let names: Vec<String> = files.iter().map(|f| f.name.clone()).collect();
    for ext in ["csv", "jsonl", "xlsx", "pdf", "parquet", "xml", "zip"] {
        assert!(
            names.iter().any(|n| n.ends_with(ext)),
            "{ext} missing from {names:?}"
        );
    }
    assert!(
        result.verification.as_str() == "ok",
        "{:?}",
        result.verification
    );
    // the bundle is classified as a bundle, the manifest as a manifest
    let bundle = files.iter().find(|f| f.name.ends_with(".zip")).unwrap();
    assert_eq!(bundle.role, ArtifactRole::Bundle);
    let manifest = files.iter().find(|f| f.name == "manifest.json").unwrap();
    assert_eq!(manifest.role, ArtifactRole::Manifest);
}

#[test]
fn the_record_cap_is_enforced_before_the_exporter_runs() {
    let d = tmp("cap");
    let p = d.join("out.jsonl");
    records(&p, 40);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let s = ExportJobSpec {
        max_records: 3,
        ..spec(&["jsonl"], true)
    };
    let (result, _) = run(&s, &cfg, &out).unwrap();
    assert_eq!(result.records, 3);
}

#[test]
fn the_export_byte_cap_is_checked_against_what_landed_on_disk() {
    let d = tmp("bytecap");
    let p = d.join("out.jsonl");
    records(&p, 30);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    let s = ExportJobSpec {
        max_export_bytes: 8,
        ..spec(&["csv", "jsonl"], false)
    };
    let e = run(&s, &cfg, &out).unwrap_err();
    assert!(e.contains("over the 8 byte cap"), "{e}");
}

#[test]
fn staging_files_are_cleaned_up_after_a_run() {
    let d = tmp("stage");
    let p = d.join("out.jsonl");
    records(&p, 5);
    let cfg = cfg_with_output(&p);
    let out = d.join("delivery");
    run(&spec(&["csv"], false), &cfg, &out).unwrap();
    assert!(!out.join(".stage").exists(), "staging dir must not ship");
}
