use super::*;

// (revalidation semantics tested in `revalidations_count_toward_no_data`)
use crate::config::AlertCfg;

fn cfg(events: &[&str], min_records: u64, max_rate: f64) -> Config {
    Config {
        alerts: AlertCfg {
            webhook_url: Some("http://127.0.0.1:1/hook".into()),
            events: events.iter().map(|s| s.to_string()).collect(),
            min_records,
            max_block_rate: max_rate,
            per_target: Default::default(),
            ..Default::default()
        },
        ..Config::default()
    }
}

#[test]
fn evaluate_projections_agree_and_quarantine_surge_is_opt_in() {
    let c = cfg(&["quarantine_surge", "no_data"], 1, 0.0);
    // threshold 0 (default): the event can never fire — legacy configs
    // see no new reasons no matter how much quarantine piles up.
    let mut s = summary(1, 0, 10);
    s.quarantined = 500;
    assert!(evaluate(&c, "ok", &s).is_empty());

    let mut c2 = c.clone();
    c2.alerts.quarantine_threshold = 10;
    let d = evaluate_detailed(&c2, "ok", &s);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].event, "quarantine_surge");
    assert_eq!(d[0].severity, Severity::Warning);
    assert_eq!(evaluate(&c2, "ok", &s), vec!["quarantine_surge:500>=10"]);
    // under threshold: silent again
    let mut s2 = summary(10, 0, 10);
    s2.quarantined = 9;
    assert!(evaluate(&c2, "ok", &s2).is_empty());
}

#[test]
fn ledger_cooldown_severity_floor_and_ack_lifecycle() {
    let cfgc = cfg(&["run_error"], 0, 0.0);
    let mut c2 = cfgc.clone();
    c2.alerts.cooldown_secs = 30;
    let err = AlertDetail {
        event: "run_error".into(),
        severity: Severity::Error,
        reason: "run_error:error".into(),
    };
    let l = AlertLedger::new();
    // first fire delivers; second fire inside the cooldown is suppressed
    assert_eq!(l.note_at(&c2, 1000, std::slice::from_ref(&err)).len(), 1);
    assert_eq!(l.note_at(&c2, 1010, std::slice::from_ref(&err)).len(), 0);
    assert_eq!(l.note_at(&c2, 1030, std::slice::from_ref(&err)).len(), 1);
    let snap = l.snapshot();
    assert_eq!(snap[0].fires, 3);
    assert_eq!(snap[0].notified, 2);
    assert_eq!(snap[0].suppressed, 1);
    // ack: errors still page (they outrank an ack by design), but a
    // resolved round wipes the whole entry — including the ack.
    assert_eq!(l.ack("run_error"), 1);
    assert_eq!(
        l.note_at(&c2, 1060, std::slice::from_ref(&err)).len(),
        1,
        "error severity beats ack"
    );
    assert_eq!(l.note_at(&c2, 1061, &[]).len(), 0);
    assert!(
        l.snapshot().is_empty(),
        "non-firing event resolves and clears ack"
    );

    // severity floor: no_data (warning) below `error` never delivers.
    let mut c3 = cfgc.clone();
    c3.alerts.min_severity = "error".into();
    let warn = AlertDetail {
        event: "no_data".into(),
        severity: Severity::Warning,
        reason: "no_data:written<1".into(),
    };
    let l3 = AlertLedger::new();
    assert!(
        l3.note_at(&c3, 1, std::slice::from_ref(&warn)).is_empty(),
        "floored by severity"
    );
    assert_eq!(l3.snapshot()[0].suppressed, 1);
    assert_eq!(l3.snapshot()[0].fires, 1, "firing is still recorded");
    // ack "*" touches live entries; unknown events report zero touched.
    assert_eq!(l3.ack("*"), 1);
    assert_eq!(l3.ack("nonexistent"), 0);
}

fn summary(written: u64, blocked: u64, requests: u64) -> Summary {
    Summary {
        written,
        requests_total: requests,
        blocked,
        ..Default::default()
    }
}

#[test]
fn block_rate_threshold_is_inclusive_and_exact() {
    let c = cfg(&["high_block_rate"], 0, 0.5);
    let just_under = evaluate(&c, "ok", &summary(51, 49, 100));
    assert!(
        just_under.is_empty(),
        "49% < 50% must not page: {just_under:?}"
    );
    let at = evaluate(&c, "ok", &summary(50, 50, 100));
    assert_eq!(
        at,
        vec!["high_block_rate:50%>=50%".to_string()],
        "the threshold is `>=` and the message shows integer percents"
    );
}

#[test]
fn quarantine_does_not_count_as_delivered_data() {
    // Deliberate semantics: a run whose records all ended up in the
    // quarantine JSONL delivered NOTHING to the sink — no_data paging
    // is correct there, not a false alarm.
    let c = cfg(&["no_data"], 1, 0.0);
    let mut s = summary(0, 0, 10);
    s.quarantined = 8;
    let r = evaluate(&c, "ok", &s);
    assert!(r.iter().any(|x| x.starts_with("no_data")), "{r:?}");
    // one real write clears it (quarantine volume never masks it either)
    let mut s2 = summary(1, 0, 10);
    s2.quarantined = 100;
    assert!(
        evaluate(&c, "ok", &s2).is_empty(),
        "written >= min => silent"
    );
}

#[tokio::test]
async fn notify_is_cheap_when_there_is_nothing_to_say_or_nowhere_to_send() {
    // empty reasons and missing webhook are both no-ops (called on the
    // hot end-of-run path by both run modes; must never fail the run).
    let c = Config::default();
    notify(&c, "ok", &[], &summary(1, 0, 1)).await;
    let mut c2 = c.clone();
    c2.alerts.webhook_url = Some("http://127.0.0.1:1/hook".into());
    c2.alerts.events = vec!["no_data".into()];
    // reasons present + dead endpoint: bounded warn, still no panic/Err
    let t0 = std::time::Instant::now();
    check_and_alert(&c2, "ok", &summary(0, 0, 0)).await;
    assert!(
        t0.elapsed() < std::time::Duration::from_secs(5),
        "notify must be bounded"
    );
}

#[test]
fn thresholds_fire_per_event() {
    let c = cfg(&["no_data", "high_block_rate"], 5, 0.5);
    let r = evaluate(&c, "ok", &summary(10, 0, 100));
    assert!(r.is_empty(), "healthy run: {r:?}");

    let r = evaluate(&c, "ok", &summary(4, 0, 100));
    assert!(r.iter().any(|x| x.starts_with("no_data")), "{r:?}");

    let r = evaluate(&c, "ok", &summary(100, 60, 100));
    assert!(r.iter().any(|x| x.starts_with("high_block_rate")), "{r:?}");

    // run_error only when subscribed
    let r = evaluate(&c, "error", &summary(100, 0, 100));
    assert!(
        !r.iter().any(|x| x.starts_with("run_error")),
        "unsubscribed event must not fire: {r:?}"
    );

    let c2 = cfg(&["run_error"], 0, 0.0);
    let r2 = evaluate(&c2, "cancelled", &summary(0, 0, 0));
    assert_eq!(r2, vec!["run_error:cancelled".to_string()]);

    // disabled => silent
    let off = Config::default();
    assert!(evaluate(&off, "error", &summary(0, 999, 999)).is_empty());
}

#[test]
fn per_target_slo_overrides_fire_independently() {
    use crate::config::TargetSlo;
    use crate::engine::TargetSummary;
    use std::collections::BTreeMap;
    // Global floor loose (1 record), global block-rate off; `shop` has a
    // real contract SLA: >=10 records and <50% blocks.
    let mut c = cfg(&["no_data", "high_block_rate"], 1, 0.0);
    c.alerts.per_target = BTreeMap::from([(
        "shop".to_string(),
        TargetSlo {
            min_records: 10,
            max_block_rate: 0.5,
        },
    )]);
    let mut s = summary(10, 0, 100); // globally healthy
    s.per_target = BTreeMap::from([(
        "shop".to_string(),
        TargetSummary {
            written: 4,
            blocked: 3,
            failed: 1,
            revalidated: 0,
            quarantined: 0,
        },
    )]);
    let r = evaluate(&c, "ok", &s);
    assert!(
        r.iter().any(|x| x == "no_data:target=shop:written<10"),
        "{r:?}"
    );
    assert!(
        r.iter().any(|x| x == "high_block_rate:target=shop"),
        "{r:?}"
    );
    assert!(
        !r.iter().any(|x| x.starts_with("no_data:written")),
        "global threshold is met; only the target SLA failed: {r:?}"
    );

    // Target recovers => silence again; unknown target ids are skipped.
    s.per_target = BTreeMap::from([(
        "shop".to_string(),
        TargetSummary {
            written: 10,
            blocked: 0,
            failed: 0,
            revalidated: 0,
            quarantined: 0,
        },
    )]);
    assert!(
        evaluate(&c, "ok", &s).is_empty(),
        "met SLA must alert nothing"
    );
    s.per_target = BTreeMap::from([(
        "shop".to_string(),
        TargetSummary {
            written: 10,
            blocked: 0,
            failed: 0,
            revalidated: 0,
            quarantined: 0,
        },
    )]);
    assert!(evaluate(&c, "ok", &s).is_empty());
}

#[test]
fn revalidations_count_toward_no_data() {
    use crate::config::TargetSlo;
    use crate::engine::TargetSummary;
    use std::collections::BTreeMap;

    // Global: an all-304 run is the healthy steady state, not "no data".
    let c = cfg(&["no_data"], 5, 0.0);
    let mut s = summary(0, 0, 10);
    s.revalidated = 7;
    assert!(
        evaluate(&c, "ok", &s).is_empty(),
        "304 steady state must stay silent"
    );
    let s0 = summary(0, 0, 10);
    assert!(
        !evaluate(&c, "ok", &s0).is_empty(),
        "0 written AND 0 revalidated must fire"
    );

    // Per-target: same arithmetic on the target's own counters.
    let mut c2 = cfg(&["no_data"], 1, 0.0);
    c2.alerts.per_target = BTreeMap::from([(
        "x".to_string(),
        TargetSlo {
            min_records: 3,
            max_block_rate: 0.0,
        },
    )]);
    let mut s2 = summary(3, 0, 10); // global fine
    s2.per_target = BTreeMap::from([(
        "x".to_string(),
        TargetSummary {
            written: 0,
            blocked: 0,
            failed: 0,
            revalidated: 2,
            quarantined: 0,
        },
    )]);
    let r = evaluate(&c2, "ok", &s2);
    assert!(
        r.iter().any(|x| x.contains("target=x")),
        "2 confirmations < 3 must fire: {r:?}"
    );
    s2.per_target = BTreeMap::from([(
        "x".to_string(),
        TargetSummary {
            written: 0,
            blocked: 0,
            failed: 0,
            revalidated: 3,
            quarantined: 0,
        },
    )]);
    assert!(
        evaluate(&c2, "ok", &s2).is_empty(),
        "3 confirmations satisfy the SLA"
    );
}
