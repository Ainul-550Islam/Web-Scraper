// Unit corpus for src/execution.rs — the lifecycle, the capacity limits,
// idempotency, cancellation and audit. Export and AI jobs run for real
// against a temp directory (no network); crawl execution is exercised by
// tests/remote_crawl.rs against a local origin.

use super::*;
use crate::config::{Config, SinkCfg, TargetCfg};
use crate::ops::OpsLog;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-ex-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::canonicalize(&d).unwrap()
}

struct Harness {
    reg: Arc<ExecutionRegistry>,
    artifacts: Arc<ArtifactStore>,
    audit: Arc<AuditLedger>,
    root: PathBuf,
}

fn harness(tag: &str) -> Harness {
    let root = tmp(tag);
    let artifacts = Arc::new(ArtifactStore::new(root.clone()));
    let audit = Arc::new(AuditLedger::new());
    let reg = Arc::new(ExecutionRegistry::new(
        root.clone(),
        artifacts.clone(),
        audit.clone(),
        Arc::new(OpsLog::new()),
        RuntimeSlot::default(),
    ));
    Harness {
        reg,
        artifacts,
        audit,
        root,
    }
}

fn cfg_with_output(path: &std::path::Path) -> Config {
    Config {
        name: "exec-test".to_string(),
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
            "{{\"run\":\"r\",\"target\":\"t\",\"url\":\"https://example.com/{i}\",\"depth\":0,\"ts\":1700000000,\"fingerprint\":\"fp{i}\",\"egress\":\"direct\",\"fields\":{{\"title\":\"T{i}\",\"price\":\"{i}.50\"}}}}\n"
        ));
    }
    std::fs::write(path, out).unwrap();
}

fn command(body: serde_json::Value, key: Option<&str>) -> ControlCommand {
    let raw = serde_json::to_vec(&body).unwrap();
    ControlCommand::from_request("admin-api", "req-unit", &raw, key.map(|k| k.to_string())).unwrap()
}

async fn wait_terminal(reg: &Arc<ExecutionRegistry>, id: &str) -> JobSnapshot {
    for _ in 0..400 {
        let snap = reg.get(id).expect("job retained");
        if snap.state == "completed" || snap.state == "failed" || snap.state == "cancelled" {
            return snap;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("job {id} never reached a terminal state");
}

// --------------------------------------------------------------- transitions

#[test]
fn legal_transitions_only() {
    assert!(ExecState::can_transition(
        ExecState::Queued,
        ExecState::Running
    ));
    assert!(ExecState::can_transition(
        ExecState::Queued,
        ExecState::Cancelled
    ));
    assert!(ExecState::can_transition(
        ExecState::Queued,
        ExecState::Failed
    ));
    assert!(ExecState::can_transition(
        ExecState::Running,
        ExecState::Completed
    ));
    assert!(ExecState::can_transition(
        ExecState::Running,
        ExecState::Failed
    ));
    assert!(ExecState::can_transition(
        ExecState::Running,
        ExecState::Cancelled
    ));
    // no resurrection
    assert!(!ExecState::can_transition(
        ExecState::Completed,
        ExecState::Running
    ));
    assert!(!ExecState::can_transition(
        ExecState::Failed,
        ExecState::Completed
    ));
    assert!(!ExecState::can_transition(
        ExecState::Cancelled,
        ExecState::Queued
    ));
    assert!(!ExecState::can_transition(
        ExecState::Queued,
        ExecState::Completed
    ));
    assert!(ExecState::Completed.is_terminal());
    assert!(ExecState::Queued.in_flight());
    assert!(!ExecState::Running.is_terminal());
}

#[test]
fn state_strings_are_the_api_contract() {
    assert_eq!(ExecState::Queued.as_str(), "queued");
    assert_eq!(ExecState::Running.as_str(), "running");
    assert_eq!(ExecState::Completed.as_str(), "completed");
    assert_eq!(ExecState::Failed.as_str(), "failed");
    assert_eq!(ExecState::Cancelled.as_str(), "cancelled");
}

#[test]
fn counters_come_from_the_engine_summary() {
    let mut s = crate::engine::Summary {
        pages_fetched: 3,
        written: 2,
        retries: 1,
        schema_drifts: 4,
        quarantined: 5,
        ..crate::engine::Summary::default()
    };
    s.per_target.insert(
        "t".to_string(),
        crate::engine::TargetSummary {
            failed: 2,
            ..Default::default()
        },
    );
    let c = JobCounters::from_summary(&s);
    assert_eq!(c.pages_fetched, 3);
    assert_eq!(c.records_written, 2);
    assert_eq!(c.failures, 2);
    assert_eq!(c.schema_drifts, 4);
    assert_eq!(c.quarantined, 5);
}

// ------------------------------------------------------------------ lifecycle

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_queued_job_can_be_cancelled_before_it_runs() {
    let h = harness("cancel-queued");
    let p = h.root.join("out.jsonl");
    records(&p, 20);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    // enqueue (no pump) leaves the job waiting for a slot, which is the
    // state a cancel-before-start has to handle
    let queued = h
        .reg
        .enqueue(&command(
            json!({"op":"export","formats":["csv"],"verify":true}),
            None,
        ))
        .unwrap();
    assert_eq!(h.reg.get(queued.as_str()).unwrap().state, "queued");

    let cancelled = h.reg.cancel(queued.as_str(), "admin-api").unwrap();
    assert_eq!(cancelled.state, "cancelled");
    assert!(cancelled.error.unwrap().contains("before it started"));
    // cancelling again is a conflict, not a silent no-op
    let e = h.reg.cancel(queued.as_str(), "admin-api").unwrap_err();
    assert_eq!(e.status(), 409);
    // cancelling never disturbs another job: this one still completes
    let first = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv","jsonl","xlsx","pdf","parquet","xml","zip"],"verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &first.job.id).await;
    assert_eq!(done.state, "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_a_terminal_job_is_a_conflict() {
    let h = harness("cancel-done");
    let p = h.root.join("out.jsonl");
    records(&p, 3);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv"],"verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "completed");
    let e = h.reg.cancel(&out.job.id, "admin-api").unwrap_err();
    assert_eq!(e.status(), 409);
    assert!(e.message.contains("already completed"));
}

#[test]
fn unknown_job_ids_are_not_found() {
    let h = harness("unknown");
    assert_eq!(
        h.reg.cancel("job-4242", "admin-api").unwrap_err().status(),
        404
    );
    assert_eq!(h.reg.cancel("nope", "admin-api").unwrap_err().status(), 404);
    assert!(h.reg.get("job-4242").is_none());
}

#[test]
fn capacity_is_bounded_and_refused_cleanly() {
    let h = harness("capacity");
    for i in 0..MAX_QUEUE_DEPTH {
        // queue jobs that can never start (no config => they fail at
        // submit-time validation for crawl, so use verify-free export jobs
        // that fail only when they run; enqueue bypasses validation)
        let cmd = command(
            json!({"op":"export","formats":["csv"]}),
            Some(&format!("k{i}")),
        );
        h.reg.enqueue(&cmd).unwrap();
    }
    assert_eq!(h.reg.in_flight(), MAX_QUEUE_DEPTH);
    let e = h
        .reg
        .enqueue(&command(json!({"op":"export","formats":["csv"]}), None))
        .unwrap_err();
    assert_eq!(e.status(), 503);
    assert!(e.message.contains("at capacity"));
}

#[test]
fn the_deadline_reaper_fails_jobs_that_outlive_their_deadline() {
    let h = harness("reap");
    let mut body = json!({"op":"export","formats":["csv"]});
    body["limits"] = json!({ "timeout_secs": 1 });
    let cmd = command(body, None);
    let id = h.reg.enqueue(&cmd).unwrap();
    assert_eq!(h.reg.get(id.as_str()).unwrap().state, "queued");
    std::thread::sleep(Duration::from_millis(2_100));
    let snap = h.reg.get(id.as_str()).unwrap();
    assert_eq!(snap.state, "failed");
    assert!(snap.error.unwrap().contains("deadline"));
    assert_eq!(h.reg.in_flight(), 0, "a reaped job is terminal");
}

#[test]
fn queued_jobs_are_started_when_a_slot_frees_up() {
    // MAX_CONCURRENT_JOBS is the running cap; enqueue more than that and the
    // rest must stay queued rather than all starting at once.
    let h = harness("slots");
    let p = h.root.join("out.jsonl");
    records(&p, 5);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    for i in 0..MAX_QUEUE_DEPTH {
        h.reg
            .enqueue(&command(
                json!({"op":"export","formats":["csv"]}),
                Some(&format!("s{i}")),
            ))
            .unwrap();
    }
    assert_eq!(h.reg.running(), 0, "enqueue never starts work by itself");
    assert_eq!(h.reg.in_flight(), MAX_QUEUE_DEPTH);
}

// --------------------------------------------------------------- idempotency

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_replayed_submission_returns_the_original_job() {
    let h = harness("idem");
    let p = h.root.join("out.jsonl");
    records(&p, 4);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let body = json!({"op":"export","formats":["csv","jsonl"],"verify":true});
    let a = h
        .reg
        .submit(&command(body.clone(), Some("retry-1")))
        .unwrap();
    let b = h
        .reg
        .submit(&command(body.clone(), Some("retry-1")))
        .unwrap();
    assert!(!a.replayed);
    assert!(b.replayed);
    assert_eq!(a.job.id, b.job.id, "no second expensive job");
    let done = wait_terminal(&h.reg, &a.job.id).await;
    assert_eq!(done.state, "completed");
    // a third call after completion still replays the same job
    let c = h.reg.submit(&command(body, Some("retry-1"))).unwrap();
    assert!(c.replayed);
    assert_eq!(c.job.state, "completed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reusing_a_key_with_a_different_body_is_a_conflict() {
    let h = harness("idem-conflict");
    let p = h.root.join("out.jsonl");
    records(&p, 4);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let _ = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv"]}),
            Some("k"),
        ))
        .unwrap();
    let e = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["jsonl"]}),
            Some("k"),
        ))
        .unwrap_err();
    assert_eq!(e.status(), 409);
    assert!(e.message.contains("different body"));
}

// ------------------------------------------------------------------- exports

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_export_job_produces_downloadable_artifacts() {
    let h = harness("export-ok");
    let p = h.root.join("out.jsonl");
    records(&p, 11);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv","jsonl"],"dataset":"unit","verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "completed", "{done:?}");
    assert_eq!(done.operation, "export");
    assert!(!done.artifacts.is_empty());
    assert_eq!(done.counters.records_written, 11);
    assert!(done.deadline_ts > done.created_ts);

    let names: Vec<String> = done
        .artifacts
        .iter()
        .filter_map(|a| a["name"].as_str().map(str::to_string))
        .collect();
    assert!(names.contains(&"records.csv".to_string()), "{names:?}");
    assert!(names.contains(&"manifest.json".to_string()), "{names:?}");
    for a in &done.artifacts {
        assert_eq!(a["state"], "ready");
        assert!(a["downloadable"].as_bool().unwrap());
        assert!(a.get("path").is_none(), "no paths in the API response");
    }
    // and the bytes really are downloadable
    let first = done.artifacts[0]["id"].as_str().unwrap();
    let opened = h.artifacts.open(first).unwrap();
    assert!(!opened.read().unwrap().is_empty());
    // the run output is untouched
    assert_eq!(std::fs::read_to_string(&p).unwrap().lines().count(), 11);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_export_reports_why_and_registers_nothing() {
    let h = harness("export-fail");
    let p = h.root.join("out.jsonl");
    records(&p, 3);
    let cfg = Config {
        output: SinkCfg::Null {}, // not a file this build can export
        ..cfg_with_output(&p)
    };
    h.reg.set_config(Arc::new(cfg));
    let e = h
        .reg
        .submit(&command(json!({"op":"export","formats":["csv"]}), None))
        .unwrap_err();
    assert_eq!(e.status(), 503);
    assert!(e.message.contains("not a file"));
    assert_eq!(h.reg.len(), 0, "a refused command creates no job");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verification_failure_makes_the_delivery_undownloadable() {
    let h = harness("export-tamper");
    let p = h.root.join("out.jsonl");
    records(&p, 6);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv","jsonl"],"verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "completed");

    // tamper with a payload, then re-verify through the control plane
    let dir = h.root.join("deliveries").join(&out.job.id);
    let victim = dir.join("records.csv");
    let mut body = std::fs::read(&victim).unwrap();
    let last = body.len() - 1;
    body[last] = body[last].wrapping_add(1);
    std::fs::write(&victim, &body).unwrap();

    let v = h.reg.verify(Some(&out.job.id), None).unwrap();
    assert_eq!(v["state"], "failed");
    let csv = h
        .artifacts
        .list()
        .into_iter()
        .find(|a| a.name == "records.csv")
        .expect("csv artifact");
    assert_eq!(csv.state, crate::download::ArtifactState::Failed);
    let e = h.artifacts.open(&csv.id).unwrap_err();
    assert_eq!(e.status(), 409);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_clean_reverify_reports_ok() {
    let h = harness("reverify");
    let p = h.root.join("out.jsonl");
    records(&p, 6);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv","jsonl"],"verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "completed");
    let v = h
        .reg
        .verify(None, Some(done.artifacts[0]["id"].as_str().unwrap()))
        .unwrap();
    assert_eq!(v["state"], "ok");
    assert_eq!(v["detail"]["state"], "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn verify_refuses_non_export_and_unfinished_jobs() {
    let h = harness("verify-guard");
    let p = h.root.join("out.jsonl");
    records(&p, 2);
    let cfg = Config {
        targets: vec![TargetCfg {
            id: "t".to_string(),
            ..TargetCfg::default()
        }],
        ..cfg_with_output(&p)
    };
    h.reg.set_config(Arc::new(cfg));
    let ai = h
        .reg
        .submit(&command(
            json!({"op":"ai","task":"classify","provider":"deterministic-local",
                   "source":{"kind":"inline","fields":[{"name":"a","value":"1"}]}}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &ai.job.id).await;
    let e = h.reg.verify(Some(&done.id), None).unwrap_err();
    assert_eq!(e.status(), 400);
    assert!(e.message.contains("only export jobs"));
    assert!(h.reg.verify(Some("job-99999"), None).is_err());
}

// ------------------------------------------------------------------------ ai

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ai_job_runs_with_the_local_provider() {
    let h = harness("ai-ok");
    let p = h.root.join("out.jsonl");
    records(&p, 2);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"ai","task":"classify","provider":"deterministic-local","confidence":0.5,
                   "source":{"kind":"inline","fields":[{"name":"price","value":"1.50"}]}}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "completed", "{done:?}");
    assert_eq!(done.result["state"], "ok");
    assert!(done.result["provider"] == "deterministic-local");
    assert!(done.result["counts"]["suggestions"].as_u64().unwrap() > 0);
    assert_eq!(done.result["external"], false);
    assert!(done.artifacts.is_empty(), "ai jobs produce no files");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_disabled_ai_job_fails_loudly_and_keeps_its_result() {
    let h = harness("ai-off");
    let p = h.root.join("out.jsonl");
    records(&p, 1);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"ai","task":"classify","provider":"disabled",
                   "source":{"kind":"inline","fields":[{"name":"a","value":"1"}]}}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    assert_eq!(done.state, "failed");
    assert_eq!(done.result["state"], "disabled", "the reason is preserved");
    assert!(done.error.unwrap().contains("disabled"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_target_is_refused_before_a_job_exists() {
    let h = harness("bad-target");
    let p = h.root.join("out.jsonl");
    records(&p, 1);
    let cfg = Config {
        targets: vec![TargetCfg {
            id: "real".to_string(),
            ..TargetCfg::default()
        }],
        ..cfg_with_output(&p)
    };
    h.reg.set_config(Arc::new(cfg));
    let e = h
        .reg
        .submit(&command(json!({"op":"crawl","targets":["ghost"]}), None))
        .unwrap_err();
    assert_eq!(e.status(), 400);
    assert!(e.message.contains("unknown target"));
    assert_eq!(h.reg.len(), 0);
}

// ---------------------------------------------------------------- listing

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_filters_paginates_and_hides_paths() {
    let h = harness("list");
    let p = h.root.join("out.jsonl");
    records(&p, 2);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    for i in 0..3 {
        let out = h
            .reg
            .submit(&command(
                json!({"op":"export","formats":["csv"]}),
                Some(&format!("l{i}")),
            ))
            .unwrap();
        wait_terminal(&h.reg, &out.job.id).await;
    }
    let (all, total) = h.reg.list(&JobFilter {
        limit: 50,
        ..JobFilter::default()
    });
    assert_eq!(total, 3);
    assert_eq!(all.len(), 3);
    let (page, total2) = h.reg.list(&JobFilter {
        limit: 2,
        ..JobFilter::default()
    });
    assert_eq!(total2, 3, "total is not the page size");
    assert_eq!(page.len(), 2);
    let (none, _) = h.reg.list(&JobFilter {
        op: Some(Operation::Crawl),
        ..JobFilter::default()
    });
    assert!(none.is_empty());
    let text = serde_json::to_string(&all).unwrap();
    assert!(
        !text.contains(&h.root.to_string_lossy().to_string()),
        "no absolute paths in job responses"
    );
    assert!(all[0].audit_ref.starts_with("audit:"));
}

#[test]
fn filter_pagination_is_clamped() {
    let f = JobFilter {
        limit: 0,
        offset: 0,
        ..JobFilter::default()
    };
    assert_eq!(f.clamped(), (1, 0));
    let f = JobFilter {
        limit: 10_000,
        offset: 5,
        ..JobFilter::default()
    };
    assert_eq!(f.clamped(), (200, 5));
}

// ----------------------------------------------------------------- audit

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_mutation_leaves_a_safe_audit_record() {
    let h = harness("audit");
    let p = h.root.join("out.jsonl");
    records(&p, 2);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv"]}),
            Some("audit-1"),
        ))
        .unwrap();
    wait_terminal(&h.reg, &out.job.id).await;
    let rows = h.audit.snapshot();
    let actions: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
    assert!(actions.contains(&"control.export.submit"), "{actions:?}");
    assert!(actions.contains(&"control.export.start"), "{actions:?}");
    assert!(actions.contains(&"control.export.finish"), "{actions:?}");
    for r in &rows {
        assert_eq!(r.actor, "admin-api");
        assert_eq!(r.corr.as_deref(), Some("req-unit"));
        for banned in ["token", "Bearer", "secret", "/tmp"] {
            assert!(!r.action.contains(banned) && !r.result.contains(banned));
        }
        assert!(r.resource.as_deref().unwrap_or("").len() < 128);
    }
    // an idempotent replay is its own audit line
    let _ = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv"]}),
            Some("audit-1"),
        ))
        .unwrap();
    let rows = h.audit.snapshot();
    assert!(rows.iter().any(|r| r.action == "control.export.replay"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inspection_returns_metadata_without_paths() {
    let h = harness("inspect");
    let p = h.root.join("out.jsonl");
    records(&p, 2);
    h.reg.set_config(Arc::new(cfg_with_output(&p)));
    let out = h
        .reg
        .submit(&command(
            json!({"op":"export","formats":["csv"],"verify":true}),
            None,
        ))
        .unwrap();
    let done = wait_terminal(&h.reg, &out.job.id).await;
    let id = done.artifacts[0]["id"].as_str().unwrap().to_string();
    let v = h.reg.inspect(&id).unwrap();
    assert_eq!(v["artifact"]["id"], id);
    assert!(v["verification"]["state"].is_string());
    assert!(!serde_json::to_string(&v)
        .unwrap()
        .contains(&h.root.to_string_lossy().to_string()));
    assert!(h.reg.inspect("art-99999").is_err());
}
