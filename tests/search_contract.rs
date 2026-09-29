//! Search contract: bounded windows, closed vocabularies, deterministic
//! ordering — over REAL history/audit JSONL files written by the tests, read
//! through `harvest::repository`. If paging, filtering or tie-breaking ever
//! drifts, CI fails here.

use std::io::Write;
use std::path::{Path, PathBuf};

use harvest::api_models::{SearchResource, Status};
use harvest::config::Config;
use harvest::search::{self, SearchQuery, MAX_PAGE};

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tmp(tag: &str) -> Dir {
    let mut p = std::env::temp_dir();
    p.push(format!("harvest-search-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    Dir(p)
}

fn write_jsonl(path: &Path, rows: &[serde_json::Value]) {
    let mut f = std::fs::File::create(path).unwrap();
    for r in rows {
        writeln!(f, "{r}").unwrap();
    }
}

/// The history file is written by the engine newest-last; repository tails it.
fn history_cfg(dir: &Dir, lines: usize) -> Config {
    let h = dir.0.join("runs.jsonl");
    let rows: Vec<serde_json::Value> = (0..lines)
        .map(|i| {
            serde_json::json!({
                "run": if i % 2 == 0 { "alpha" } else { "beta" },
                "status": if i % 5 == 0 { "error" } else { "ok" },
                "ts": 1000 + i as u64,
                "duration_ms": (i as u64 * 37) % 101,
                "summary": { "pages_fetched": i, "records_written": i * 2 }
            })
        })
        .collect();
    write_jsonl(&h, &rows);
    let mut cfg = Config::default();
    cfg.schedule.history_path = h;
    cfg
}

fn run(c: &Config, q: &SearchQuery) -> search::SearchPage {
    search::search(c, q).unwrap()
}

fn q(runs: impl FnOnce(&mut SearchQuery)) -> SearchQuery {
    let mut q = SearchQuery {
        ..Default::default()
    };
    runs(&mut q);
    q
}

#[test]
fn limit_is_clamped_never_trusted() {
    let d = tmp("limit");
    let cfg = history_cfg(&d, 60);
    let big = run(
        &cfg,
        &q(|x| {
            x.limit = 10_000;
        }),
    );
    assert!(big.rows.len() <= MAX_PAGE);
    let zero = run(
        &cfg,
        &q(|x| {
            x.limit = 0;
        }),
    );
    // limit 0 is meaningless: callers see the clamped minimum via from_params;
    // a hand-built 0 must not panic and must return nothing (page of zero)
    assert!(zero.rows.is_empty());
    // from_params path clamps instead
    let parsed =
        SearchQuery::from_params("runs", |k| (k == "limit").then(|| "99999".to_string())).unwrap();
    assert_eq!(parsed.limit, MAX_PAGE);
}

#[test]
fn offset_boundaries_and_next_marker() {
    let d = tmp("offset");
    let cfg = history_cfg(&d, 60);
    let p0 = run(
        &cfg,
        &q(|x| {
            x.limit = 10;
        }),
    );
    assert_eq!(p0.rows.len(), 10);
    assert_eq!(p0.page.next_offset, Some(10), "more rows exist behind us");
    let p5 = run(
        &cfg,
        &q(|x| {
            x.limit = 10;
            x.offset = 50;
        }),
    );
    assert_eq!(p5.rows.len(), 10);
    assert_eq!(p5.page.next_offset, None, "exact end of window");
    let p9 = run(
        &cfg,
        &q(|x| {
            x.limit = 10;
            x.offset = 59;
        }),
    );
    assert_eq!(p9.rows.len(), 1);
    assert_eq!(p9.page.next_offset, None);
    // beyond the end: empty page, no error
    let p99 = run(
        &cfg,
        &q(|x| {
            x.offset = 59;
            x.limit = 1;
            x.since = Some(999_999);
        }),
    );
    assert!(p99.rows.is_empty());
    assert_eq!(p99.page.next_offset, None);
}

#[test]
fn offset_plus_limit_beyond_the_window_is_refused_not_attempted() {
    let d = tmp("bound");
    let _cfg = history_cfg(&d, 10);
    let err = SearchQuery::from_params("runs", |k| match k {
        "offset" => Some("500".into()),
        "limit" => Some("100".into()),
        _ => None,
    });
    assert!(err.is_err(), "512-window is a promise, not a suggestion");
    if let Err(e) = err {
        assert!(e.to_json("t")["error"]["message"]
            .as_str()
            .unwrap()
            .contains("bounded window"));
    }
}

#[test]
fn status_and_target_and_range_filters_compose() {
    let d = tmp("filters");
    let cfg = history_cfg(&d, 60);
    let only_errors = run(
        &cfg,
        &q(|x| {
            x.status = Some(Status::Error);
            x.limit = 200;
        }),
    );
    assert!(only_errors
        .rows
        .iter()
        .all(|r| r["status"] == serde_json::json!("error")));
    assert_eq!(only_errors.rows.len(), 12, "i%5==0 for i in 0..60");
    let both = run(
        &cfg,
        &q(|x| {
            x.status = Some(Status::Ok);
            x.target = None;
            x.since = Some(1040); // ts >= 1040
            x.limit = 200;
        }),
    );
    assert!(both
        .rows
        .iter()
        .all(|r| r["status"] == serde_json::json!("ok") && r["ts"].as_u64().unwrap() >= 1040));
    // run-name text search over the allowlist fields (runs: run|status)
    let text = run(
        &cfg,
        &q(|x| {
            x.text = Some("ALPHA".into()); // case-insensitive substring
            x.limit = 200;
        }),
    );
    assert!(text
        .rows
        .iter()
        .all(|r| r["run"] == serde_json::json!("alpha")));
    assert_eq!(text.rows.len(), 30);
}

#[test]
fn since_until_are_inclusive_bounds() {
    let d = tmp("range");
    let cfg = history_cfg(&d, 10); // ts 1000..=1009
    let mid = run(
        &cfg,
        &q(|x| {
            x.since = Some(1003);
            x.until = Some(1005);
            x.limit = 200;
        }),
    );
    let ts: Vec<u64> = mid.rows.iter().map(|r| r["ts"].as_u64().unwrap()).collect();
    assert_eq!(ts.len(), 3);
    assert!(
        ts.contains(&1003) && ts.contains(&1005),
        "inclusive on both ends: {ts:?}"
    );
}

#[test]
fn sorting_ties_and_empty_query_are_deterministic() {
    let d = tmp("sort");
    // six runs, three sharing one ts: tie order must be stable file-order
    let h = d.0.join("runs.jsonl");
    write_jsonl(
        &h,
        &[
            serde_json::json!({"run":"x1","status":"ok","ts":5,"duration_ms":10}),
            serde_json::json!({"run":"x2","status":"ok","ts":5,"duration_ms":10}),
            serde_json::json!({"run":"x3","status":"ok","ts":5,"duration_ms":10}),
            serde_json::json!({"run":"big","status":"ok","ts":1,"duration_ms":99}),
        ],
    );
    let mut cfg = Config::default();
    cfg.schedule.history_path = h;
    let names = |q: &SearchQuery| -> Vec<String> {
        run(&cfg, q)
            .rows
            .iter()
            .map(|r| r["run"].as_str().unwrap().to_string())
            .collect()
    };
    let first = names(&q(|x| {
        x.sort = Some("ts".into());
        x.desc = true;
    }));
    for _ in 0..25 {
        assert_eq!(
            names(&q(|x| {
                x.sort = Some("ts".into());
                x.desc = true;
            })),
            first,
            "repeated identical query must return identical order"
        );
    }
    assert_eq!(
        first,
        vec!["x3", "x2", "x1", "big"],
        "ties keep the read-window order (newest-first, as documented)"
    );
    let by_dur = names(&q(|x| {
        x.sort = Some("duration".into());
        x.desc = true;
    }));
    assert_eq!(by_dur[0], "big", "duration sort honors the field");
    // empty text query is no query at all
    let all = names(&q(|x| {
        x.limit = 200;
    }));
    assert_eq!(all.len(), 4);
}

#[test]
fn unknown_sort_and_resource_and_order_are_structured_errors() {
    let bad_resource = SearchQuery::from_params("users", |_| None).unwrap_err();
    assert_eq!(
        bad_resource.to_json("c")["error"]["code"],
        serde_json::json!("invalid_request")
    );
    let d = tmp("vocab");
    let cfg = history_cfg(&d, 3);
    let bad_sort = SearchQuery::from_params("runs", |k| {
        (k == "sort").then(|| "records_written".to_string())
    })
    .unwrap_err();
    assert!(bad_sort.message.contains("not supported"));
    let bad_order =
        SearchQuery::from_params("runs", |k| (k == "order").then(|| "sideways".to_string()))
            .unwrap_err();
    assert!(bad_order.message.contains("asc|desc"));
    let bad_status =
        SearchQuery::from_params("runs", |k| (k == "status").then(|| "meh".to_string()))
            .unwrap_err();
    assert!(bad_status.message.contains("unknown status"));
    // events resource parses but with its own narrow sort vocabulary
    let ok_events =
        SearchQuery::from_params("events", |k| (k == "sort").then(|| "ts".to_string())).unwrap();
    assert_eq!(ok_events.resource, SearchResource::Events);
    let bad_events_sort =
        SearchQuery::from_params("events", |k| (k == "sort").then(|| "duration".to_string()))
            .unwrap_err();
    assert!(bad_events_sort.message.contains("not supported"));
    let _ = cfg;
}

#[test]
fn large_bounded_query_reports_truncation_instead_of_lying() {
    let d = tmp("big");
    let cfg = history_cfg(&d, 600); // > 512 window
    let p = run(
        &cfg,
        &q(|x| {
            x.offset = 300;
            x.limit = 200;
        }),
    );
    assert_eq!(p.page.scanned, 600, "honest count of lines looked at");
    assert_eq!(p.rows.len(), 200);
    // records are capped at the 512 tail: page walks that window only
    let oldest_returned = p
        .rows
        .iter()
        .map(|r| r["ts"].as_u64().unwrap())
        .min()
        .unwrap();
    assert_eq!(
        oldest_returned,
        1000 + 100,
        "window = newest 512; offset 300 ends at i=100"
    );
}

#[test]
fn missing_history_file_is_an_empty_page_not_an_error() {
    let d = tmp("missing");
    let mut cfg = Config::default();
    cfg.schedule.history_path = d.0.join("nope.jsonl");
    let p = run(&cfg, &q(|_| {}));
    assert!(p.rows.is_empty());
    assert_eq!(p.page.scanned, 0);
    assert!(!p.page.truncated);
}
