use super::*;

#[test]
fn history_entry_shape_and_record_append() {
    let dir = std::env::temp_dir().join(format!("harvest-runner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = Config::default();
    cfg.schedule.history_path = dir.join("nested/runs.jsonl");
    let summary = Summary {
        pages_fetched: 3,
        requests_total: 3,
        records: 2,
        written: 2,
        deduped: 1,
        blocked: 0,
        retries: 0,
        robots_denied: 0,
        ckpt_writes: 0,
        revalidated: 0,
        quarantined: 0,
        schema_drifts: 0,
        duration_ms: 42,
        per_target: Default::default(),
    };
    let e1 = history_entry(&cfg, &summary, "ok");
    assert_eq!(e1["status"], "ok");
    assert_eq!(e1["summary"]["pages_fetched"], 3);
    assert_eq!(e1["config_sha256"].as_str().unwrap().len(), 64);
    record_history(&cfg, &e1).unwrap();
    let e2 = history_entry(&cfg, &summary, "error");
    record_history(&cfg, &e2).unwrap();
    let txt = std::fs::read_to_string(&cfg.schedule.history_path).expect("history file must exist");
    let lines: Vec<&str> = txt.lines().collect();
    assert_eq!(lines.len(), 2, "appends must accumulate");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn history_entry_carries_pipeline_counters() {
    // The scheduler log is the operator's only cross-run view of schema
    // health: quarantined/drift totals must survive serialization.
    let cfg = Config {
        name: "pipe".into(),
        ..Default::default()
    };
    let summary = Summary {
        pages_fetched: 4,
        requests_total: 4,
        records: 4,
        written: 1,
        deduped: 0,
        blocked: 0,
        retries: 0,
        robots_denied: 0,
        ckpt_writes: 0,
        revalidated: 0,
        quarantined: 3,
        schema_drifts: 7,
        duration_ms: 9,
        per_target: Default::default(),
    };
    let e = history_entry(&cfg, &summary, "ok");
    assert_eq!(e["summary"]["quarantined"], 3);
    assert_eq!(e["summary"]["schema_drifts"], 7);
    assert_eq!(e["run"], "pipe");
}
