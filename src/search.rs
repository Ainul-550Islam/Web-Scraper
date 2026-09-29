//! Bounded operational search over data the engine already persists — run
//! history and the audit log — through [`crate::repository`] windows. No
//! index, no database, no whole-dataset scans: every query sees at most the
//! repository scan budget and says so (`PageEcho.truncated`).
//!
//! Semantics (pinned by `tests/search_contract.rs`):
//! * filters: resource, status (api::Status vocabulary), target,
//!   since/until epoch range, free-text over a FIXED allowlist of string
//!   fields (run/target/status fields — never a scan of record payloads);
//! * sorting: by the fields the sources actually carry (ts, duration for
//!   runs; ts for audit) with a deterministic tie-breaker: equal values keep
//!   the read-window order (newest-first, as the repository returns it), so
//!   equal values never reorder between runs;
//! * paging: limit clamped to 1..=MAX_PAGE (200), offset ≥ 0; the window is
//!   re-tailed deeper for offset>0 but still bounded (offset+limit ≤ scan cap).

use serde_json::Value;

use crate::api_models::{ApiError, PageEcho, SearchResource, Status};
use crate::repository::{self, Window, MAX_RECORDS_PER_QUERY};

/// Max rows one page may return.
pub const MAX_PAGE: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchQuery {
    pub resource: SearchResource,
    pub text: Option<String>,
    pub status: Option<Status>,
    pub target: Option<String>,
    pub since: Option<u64>,
    pub until: Option<u64>,
    /// "ts" | "duration" (runs) or "ts" (events); default = source order
    pub sort: Option<String>,
    pub desc: bool,
    pub limit: usize,
    pub offset: usize,
}

impl Default for SearchQuery {
    /// runs newest-first, limit 50 — the API's documented list defaults.
    fn default() -> Self {
        Self {
            resource: SearchResource::Runs,
            text: None,
            status: None,
            target: None,
            since: None,
            until: None,
            sort: None,
            desc: true,
            limit: 50,
            offset: 0,
        }
    }
}

impl SearchQuery {
    /// Parse from raw query params with the engine's clamping philosophy
    /// (limit 1..=MAX_PAGE, never negative, unknown sort refused).
    pub fn from_params(
        resource: &str,
        mut p: impl FnMut(&str) -> Option<String>,
    ) -> Result<Self, ApiError> {
        let mut q = SearchQuery {
            resource: SearchResource::parse(resource)?,
            ..Default::default()
        };
        let mut ints: Vec<(String, u64)> = Vec::new();
        for key in ["limit", "offset", "since", "until"] {
            if let Some(v) = p(key) {
                let n = v.parse::<u64>().map_err(|_| {
                    ApiError::invalid(format!("`{key}` must be a non-negative integer"))
                })?;
                ints.push((key.to_string(), n));
            }
        }
        for (k, v) in ints {
            match k.as_str() {
                "limit" => q.limit = (v as usize).clamp(1, MAX_PAGE),
                "offset" => q.offset = v as usize,
                "since" => q.since = Some(v),
                "until" => q.until = Some(v),
                _ => unreachable!(),
            }
        }
        if let Some(s) = p("status") {
            q.status = Some(Status::parse(&s)?);
        }
        if let Some(t) = p("target") {
            q.target = Some(truncate(&t, 128));
        }
        if let Some(t) = p("q") {
            let t = t.trim().to_string();
            if !t.is_empty() {
                q.text = Some(truncate(&t, 64));
            }
        }
        if let Some(s) = p("sort") {
            match (q.resource, s.as_str()) {
                (SearchResource::Runs, "ts" | "duration") | (SearchResource::Events, "ts") => {
                    q.sort = Some(s)
                }
                (_, other) => {
                    return Err(ApiError::invalid(format!(
                        "sort `{other}` not supported for resource `{}`",
                        q.resource.as_str()
                    )))
                }
            }
        }
        if let Some(o) = p("order") {
            match o.as_str() {
                "desc" => q.desc = true,
                "asc" => q.desc = false,
                other => {
                    return Err(ApiError::invalid(format!(
                        "unknown order `{other}` (asc|desc)"
                    )))
                }
            }
        }
        if q.offset + q.limit > MAX_RECORDS_PER_QUERY {
            return Err(ApiError::invalid(format!(
                "offset+limit exceeds the bounded window ({MAX_RECORDS_PER_QUERY})"
            )));
        }
        Ok(q)
    }
}

fn truncate(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    t
}

#[derive(Debug)]
pub struct SearchPage {
    pub rows: Vec<Value>,
    pub page: PageEcho,
}

/// Run a query against the given config's files. Deterministic: same files +
/// same query ⇒ same rows, byte for byte. The audit REPORT file is not a
/// search resource (single document — read it via
/// [`crate::repository::audit_report`]).
pub fn search(cfg: &crate::config::Config, q: &SearchQuery) -> std::io::Result<SearchPage> {
    let want = (q.offset + q.limit + 64).min(MAX_RECORDS_PER_QUERY);
    let win: Window = match q.resource {
        SearchResource::Runs => repository::history_records(cfg, want)?,
        SearchResource::Events => repository::events_records(cfg, want)?,
    };
    Ok(apply_filters(&win, q))
}

/// Pure filter/sort/page step (exposed for tests: file I/O and query
/// semantics stay independently checkable).
pub fn apply_filters(win: &Window, q: &SearchQuery) -> SearchPage {
    let mut matched: Vec<(usize, Value)> = Vec::new();
    for (idx, rec) in win.records.iter().enumerate() {
        if let Some(st) = q.status {
            // events carry no run-status vocabulary — from_params refuses
            // this combination; a directly-built query simply does not match
            // records that have no `status` field.
            if rec["status"].as_str() != Some(st.as_str()) {
                continue;
            }
        }
        if let Some(t) = &q.target {
            if rec["target"].as_str() != Some(t.as_str())
                && rec["target_id"].as_str() != Some(t.as_str())
            {
                continue;
            }
        }
        let ts = rec_ts(q.resource, rec);
        if let Some(since) = q.since {
            if ts < since {
                continue;
            }
        }
        if let Some(until) = q.until {
            if ts > until {
                continue;
            }
        }
        if let Some(text) = &q.text {
            if !text_match(q.resource, rec, text) {
                continue;
            }
        }
        matched.push((idx, rec.clone()));
    }

    if let Some(key) = &q.sort {
        let rank = |r: &Value| -> (i128, u64) {
            let v = match key.as_str() {
                "duration" => r["duration_ms"].as_i64().unwrap_or(i64::MIN) as i128,
                _ => ts_i128(r),
            };
            (v, 0)
        };
        matched.sort_by(|(ia, a), (ib, b)| {
            let (ra, _) = rank(a);
            let (rb, _) = rank(b);
            let ord = if q.desc { rb.cmp(&ra) } else { ra.cmp(&rb) };
            match ord {
                std::cmp::Ordering::Equal => ia.cmp(ib), // tie: window order
                o => o,
            }
        });
    } else if !q.desc {
        // source order is newest-first; `asc` flips it deterministically
        matched.reverse();
    }

    let total_matched = matched.len();
    let rows: Vec<Value> = matched
        .into_iter()
        .skip(q.offset)
        .take(q.limit)
        .map(|(_, v)| v)
        .collect();
    let returned = rows.len();
    let next = if q.offset + returned < total_matched {
        Some(q.offset + returned)
    } else {
        None
    };
    SearchPage {
        rows,
        page: PageEcho {
            limit: q.limit,
            offset: q.offset,
            returned,
            scanned: win.stats.lines_scanned,
            truncated: win.stats.truncated,
            malformed_lines: win.stats.malformed_lines,
            next_offset: next,
        },
    }
}

fn ts_i128(r: &Value) -> i128 {
    r["ts"].as_u64().or_else(|| r["ts"].as_u64()).unwrap_or(0) as i128
}

fn rec_ts(res: SearchResource, r: &Value) -> u64 {
    match res {
        // history rows (runner::history_entry) carry "ts" — seconds epoch —
        // with "ts" kept as a legacy fallback for hand-written files
        SearchResource::Runs => ts_i128(r).max(0) as u64,
        SearchResource::Events => r["ts"].as_u64().unwrap_or(0),
    }
}

/// Text search over a fixed allowlist of fields (never payload blobs — the
/// point is bounded, predictable cost).
fn text_match(res: SearchResource, r: &Value, text: &str) -> bool {
    let needle = text.to_lowercase();
    let fields: &[&str] = match res {
        SearchResource::Runs => &["run", "status"],
        SearchResource::Events => &["kind"],
    };
    fields.iter().any(|f| {
        r[*f]
            .as_str()
            .is_some_and(|v| v.to_lowercase().contains(&needle))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn window(vals: Vec<Value>) -> Window {
        use std::path::PathBuf;
        let n = vals.len();
        Window {
            records: vals,
            stats: repository::ScanStats {
                lines_scanned: n,
                ..Default::default()
            },
            path: PathBuf::from("mem"),
        }
    }

    fn q(overrides: impl FnOnce(&mut SearchQuery)) -> SearchQuery {
        let mut q = SearchQuery {
            resource: SearchResource::Runs,
            limit: 50,
            ..Default::default()
        };
        overrides(&mut q);
        q
    }

    #[test]
    fn newest_first_default_and_asc_flip_are_deterministic() {
        let w = window(vec![
            json!({"run": "a", "ts": 3}),
            json!({"run": "b", "ts": 2}),
            json!({"run": "c", "ts": 1}),
        ]);
        let p = apply_filters(&w, &q(|_| {}));
        let runs: Vec<_> = p.rows.iter().map(|r| r["run"].as_str().unwrap()).collect();
        assert_eq!(runs, vec!["a", "b", "c"]);
        let p2 = apply_filters(
            &w,
            &q(|x| {
                x.desc = false;
            }),
        );
        let runs2: Vec<_> = p2.rows.iter().map(|r| r["run"].as_str().unwrap()).collect();
        assert_eq!(runs2, vec!["c", "b", "a"]);
    }

    #[test]
    fn ties_break_on_file_order_not_hash_noise() {
        let w = window(vec![
            json!({"run": "x", "ts": 5}),
            json!({"run": "y", "ts": 5}),
            json!({"run": "z", "ts": 5}),
        ]);
        for _ in 0..5 {
            let p = apply_filters(
                &w,
                &q(|x| {
                    x.sort = Some("ts".into());
                    x.desc = true;
                }),
            );
            let order: Vec<_> = p.rows.iter().map(|r| r["run"].as_str().unwrap()).collect();
            assert_eq!(order, vec!["x", "y", "z"], "stable tie-breaker");
        }
    }

    #[test]
    fn filters_compose_and_page_echo_reports_bounds() {
        let w = window(vec![
            json!({"run": "r1", "status": "ok", "ts": 100}),
            json!({"run": "r2", "status": "error", "ts": 90}),
            json!({"run": "r1", "status": "ok", "ts": 80}),
        ]);
        let p = apply_filters(
            &w,
            &q(|x| {
                x.status = Some(Status::Ok);
                x.since = Some(85);
            }),
        );
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.rows[0]["run"], json!("r1"));
        assert_eq!(p.page.next_offset, None);
        assert!(!p.page.truncated);
    }

    #[test]
    fn from_params_clamps_and_refuses_hostile_values() {
        let mut params = std::collections::BTreeMap::<String, String>::new();
        params.insert("limit".into(), "999999".into());
        params.insert("offset".into(), "0".into());
        params.insert("sort".into(), "ts".into());
        let q = SearchQuery::from_params("runs", |k| params.get(k).cloned()).unwrap();
        assert_eq!(q.limit, MAX_PAGE, "limit clamped, not trusted");
        params.insert("sort".into(), "password".into());
        assert!(SearchQuery::from_params("runs", |k| params.get(k).cloned()).is_err());
        params.insert("sort".into(), "ts".into());
        params.insert("offset".into(), "9999".into());
        assert!(
            SearchQuery::from_params("runs", |k| params.get(k).cloned()).is_err(),
            "offset+limit beyond the bounded window is refused, not attempted"
        );
    }
}
