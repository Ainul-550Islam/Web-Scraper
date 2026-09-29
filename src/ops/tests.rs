use super::*;

use serde_json::json;

#[test]
fn rings_evict_at_their_caps_and_keep_order() {
    let log = OpsLog::new();
    for i in 0..(OPS_CAP + 10) {
        log.record(&RuntimeSlot::default(), "run", json!({"i": i}));
    }
    let snap = log.snapshot();
    assert_eq!(snap.len(), OPS_CAP, "hard cap");
    assert_eq!(snap[0].detail["i"], (10u64), "oldest 10 evicted");
    assert_eq!(snap.last().unwrap().detail["i"], (OPS_CAP + 9) as u64);
    let tail = log.tail(3);
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0].detail["i"], (OPS_CAP + 9) as u64, "newest first");

    let audit = AuditLedger::new();
    for i in 0..(AUDIT_CAP + 4) {
        audit.note(
            "admin-api",
            "alerts.ack",
            "ok",
            Some("no_data"),
            &format!("c{i}"),
        );
    }
    assert_eq!(audit.snapshot().len(), AUDIT_CAP);
    assert_eq!(audit.tail(1)[0].corr.as_deref(), Some("c131"));
    assert_eq!(audit.snapshot()[0].corr.as_deref(), Some("c4"));
}

#[test]
fn sanitize_blocks_log_forgery_and_bounds_length() {
    assert_eq!(sanitize("ok-line_1", 64), "ok-line_1");
    let evil = "ack\r\nINFO fake entry";
    let clean = sanitize(evil, 64);
    assert!(!clean.contains('\r') && !clean.contains('\n'), "{clean:?}");
    let long = "x".repeat(500);
    assert_eq!(sanitize(&long, 64).chars().count(), 64);
    assert_eq!(sanitize("a\u{7}b", 64), "a?b");
}

#[test]
fn ops_events_mirror_to_attached_runtime_bus() {
    // A runtime attached to the slot must see the operator action on the
    // engine bus; a detached slot records locally and emits nothing.
    use crate::metrics::Metrics;
    use crate::runtime::Runtime;
    use std::sync::Arc;
    let cfg = crate::config::Config::default();
    let rt = Runtime::new(&cfg, Arc::new(Metrics::default()));
    let slot = RuntimeSlot::default();
    slot.set(rt.clone());
    let log = OpsLog::new();
    log.record(&slot, "alerts.ack", json!({"event": "no_data"}));
    let tail = rt.bus.tail(8);
    assert_eq!(tail.len(), 1, "{tail:?}");
    assert_eq!(tail[0].kind, EventKind::OperatorAction);
    assert_eq!(tail[0].detail["kind"], "alerts.ack");
    assert_eq!(log.snapshot()[0].detail["event"], "no_data");
    // detached slot: recorded locally, bus untouched, no panic
    let detached = OpsLog::new();
    detached.record(&RuntimeSlot::default(), "x", json!({}));
    assert_eq!(detached.snapshot().len(), 1);
    assert!(
        rt.bus.tail(8).len() == 1,
        "detached log emitted nothing new"
    );
}
