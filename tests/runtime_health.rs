//! Runtime lifecycle + health domain, integration level: transitions,
//! shutdown finality, degradation signalling, dependency classification,
//! storage-lane verdicts and the exact vocabulary `harvest health` renders.
//! Everything here runs against real types — no mocks of our own modules.

use std::sync::Arc;
use std::time::Duration;

use harvest::api::ComponentStatus;
use harvest::config::Config;
use harvest::health;
use harvest::metrics::Metrics;
use harvest::runtime::{LifecycleState, Runtime};
use harvest::supervisor::{CancelToken, RestartPolicy, Supervisor};
use serde_json::json;

fn fresh_runtime() -> Arc<Runtime> {
    let cfg = Config::default();
    Runtime::new(&cfg, Arc::new(Metrics::default()))
}

#[test]
fn startup_to_running_transition() {
    let rt = fresh_runtime();
    assert_eq!(rt.lifecycle(), LifecycleState::Starting);
    // uptime is monotonic and never negative-jumpy
    std::thread::sleep(Duration::from_millis(5));
    assert!(rt.uptime_ms() >= 4);
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    rt.attach_supervisor(sup);
    assert_eq!(rt.lifecycle(), LifecycleState::Running);
    // attach is idempotent-safe for the state machine (no re-Starting)
    assert_eq!(rt.lifecycle_json()["state"], json!("running"));
    assert!(rt.started_unix() > 0);
}

#[test]
fn degrade_only_from_running_and_recovers_back() {
    let rt = fresh_runtime();
    assert!(!rt.mark_degraded(), "cannot degrade before running");
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    rt.attach_supervisor(sup);
    assert!(rt.mark_degraded());
    assert_eq!(rt.lifecycle(), LifecycleState::Degraded);
    assert!(rt.clear_degraded());
    assert_eq!(rt.lifecycle(), LifecycleState::Running);
    assert!(!rt.clear_degraded(), "no double-clear");
}

#[test]
fn draining_then_stopped_and_stopped_is_final() {
    let rt = fresh_runtime();
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    rt.attach_supervisor(sup);
    rt.note_shutdown_reason("signal");
    rt.begin_shutdown();
    assert_eq!(rt.lifecycle(), LifecycleState::Draining);
    let view = rt.lifecycle_json();
    assert!(view["drain"].is_object(), "drain marker visible");
    assert_eq!(view["drain"]["reason"], json!("signal"));
    // re-entering drain keeps the FIRST timestamp (no jitter)
    let t1 = view["drain"]["at_ms"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(2));
    rt.begin_shutdown();
    assert_eq!(rt.lifecycle_json()["drain"]["at_ms"].as_u64(), Some(t1));
    rt.end_shutdown(&json!({"written": 0}));
    assert_eq!(rt.lifecycle(), LifecycleState::Stopped);
    // STOPPED IS FINAL: another begin must not reopen it
    rt.begin_shutdown();
    assert_eq!(
        rt.lifecycle(),
        LifecycleState::Stopped,
        "stopped is terminal"
    );
    // snapshot() stays exactly the historical public shape:
    let snap = rt.snapshot(true);
    assert!(
        snap.get("lifecycle").is_none(),
        "lifecycle must NOT leak into GET /runtime — it has its own view"
    );
}

#[test]
fn shutdown_reason_vocabulary_is_bounded() {
    let rt = fresh_runtime();
    rt.note_shutdown_reason("operator typed whatever long string here");
    rt.begin_shutdown();
    let reason = rt.lifecycle_json()["drain"]["reason"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(reason, "signal", "unbounded input falls back, never echoes");
    let rt2 = fresh_runtime();
    for ok in ["signal", "schedule", "admin", "fault"] {
        rt2.note_shutdown_reason(ok);
    }
    rt2.begin_shutdown();
    assert_eq!(
        rt2.lifecycle_json()["drain"]["reason"],
        json!("fault"),
        "last valid reason wins"
    );
}

#[test]
fn dependency_classification_distinguishes_disabled_from_unavailable() {
    use harvest::health::classify;
    assert_eq!(classify(false, false), ComponentStatus::Disabled);
    assert_eq!(classify(true, false), ComponentStatus::Unavailable);
    assert_eq!(classify(true, true), ComponentStatus::Healthy);
    // aggregation never lets "all disabled" escalate
    let rep = health::aggregate(vec![harvest::api::ComponentReport {
        name: "redis",
        status: ComponentStatus::Disabled,
        detail: None,
    }]);
    assert_eq!(rep.overall, ComponentStatus::Healthy);
}

#[test]
fn storage_lane_probe_covers_every_lane_with_stable_keys() {
    let cfg = Config::default();
    let (checks, degraded) = health::lane_checks(&cfg);
    assert!(!degraded);
    let names: Vec<&str> = checks.iter().map(|c| c["lane"].as_str().unwrap()).collect();
    for lane in [
        "output",
        "history",
        "events",
        "audit",
        "quarantine",
        "checkpoint",
        "revalidation",
    ] {
        assert!(
            names.contains(&lane),
            "lane {lane} missing — /storage parity broken"
        );
    }
    // statuses come only from the documented vocabulary
    for c in &checks {
        assert!(
            ["healthy", "disabled", "degraded"].contains(&c["status"].as_str().unwrap()),
            "{c:?}"
        );
    }
}

#[test]
fn quarantine_lane_with_records_degrades_the_probe_end_to_end() {
    let dir = std::env::temp_dir().join(format!("harvest-rh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let q = dir.join("q.jsonl");
    std::fs::write(&q, "{\"reason\":\"missing field\"}\n").unwrap();
    let json = json!({
        "name": "rh",
        "targets": [{
            "id": "t", "start_url": "http://127.0.0.1:1/",
            "schema": { "enforce": "strict", "quarantine_path": q.to_str().unwrap() }
        }]
    });
    let cfg: Config = serde_json::from_value(json).unwrap();
    let (checks, degraded) = health::lane_checks(&cfg);
    assert!(
        degraded,
        "non-empty quarantine lane must flip the flag the CLI bails on"
    );
    let row = checks
        .iter()
        .find(|c| c["lane"] == json!("quarantine"))
        .unwrap();
    assert_eq!(row["status"], json!("degraded"));
    assert_eq!(row["detail"], json!("quarantine lane has records"));
    let _ = std::fs::remove_dir_all(&dir);
}
