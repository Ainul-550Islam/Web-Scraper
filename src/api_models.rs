//! Shared API/domain models for read-side features — ONLY the pieces that
//! were missing; everything that already exists is re-exported, never
//! restated. This is the file a new consumer imports instead of inventing
//! its own query/paging/error shapes.
//!
//! Re-exports (single source of truth stays in [`crate::api`] /
//! [`crate::health`]):
//! * `ApiError` (structured error + `to_json` correlation envelope),
//! * `Page` (limit/offset with server-side clamps), `ListFilter`, `Status`,
//!   `SortKey`, `SortDir` (the bounded sort vocabulary),
//! * `ComponentStatus`, `ComponentReport`, `HealthReport` (health wire types).
//!
//! New here:
//! * `SearchOrder`/`SearchResource` — closed vocabularies consumed by
//!   [`crate::search`] (and any future `/search` surface);
//! * `PageEcho` — the pagination envelope read APIs return (what you asked,
//!   what we give, whether the source truncated);
//! * `ErrorReply` — helper producing the exact `ApiError::to_json` shape for
//!   library callers that are not HTTP handlers, so their errors look like
//!   the API's errors.

pub use crate::api::{
    ApiError, ComponentReport, ComponentStatus, HealthReport, ListFilter, Page, SortDir, SortKey,
    Status,
};

use serde::{Deserialize, Serialize};

/// The resources the operational search layer can query. Closed set — adding
/// one means also wiring its repository source and contract tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchResource {
    /// run-history JSONL (schedule.history_path — `GET /runs` reads the same
    /// file)
    Runs,
    /// structured event log JSONL (events_path — `GET /events` tails the same
    /// in-memory ring while a run is live)
    Events,
}

impl SearchResource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runs => "runs",
            Self::Events => "events",
        }
    }
    pub fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "runs" => Ok(Self::Runs),
            "events" => Ok(Self::Events),
            other => Err(ApiError::invalid(format!(
                "unknown resource `{other}` (allowed: runs, events)"
            ))),
        }
    }
}

/// Pagination envelope for list/search replies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PageEcho {
    pub limit: usize,
    pub offset: usize,
    /// records returned in this page
    pub returned: usize,
    /// records the query actually looked at (≤ MAX_RECORDS_PER_QUERY)
    pub scanned: usize,
    /// true when the source had more records than the scan budget — the UI
    /// must show this rather than implying a full scan
    pub truncated: bool,
    /// malformed source lines skipped (never silently dropped)
    pub malformed_lines: usize,
    /// offset to pass for the next page, or None at the end
    pub next_offset: Option<usize>,
}

/// Structured error reply for non-HTTP library callers — byte-shape of the
/// admin API body: `{"error": {code, message}, "request_id": …}`.
pub fn error_reply(err: &ApiError, correlation: &str) -> serde_json::Value {
    err.to_json(correlation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_vocabulary_is_closed_and_exact() {
        assert_eq!(SearchResource::parse("runs").unwrap(), SearchResource::Runs);
        assert_eq!(
            SearchResource::parse("events").unwrap(),
            SearchResource::Events
        );
        let e = SearchResource::parse("users").unwrap_err();
        // structured error, honest message, no echo of hostile input beyond the bounded name
        let v = error_reply(&e, "corr-1");
        // the API's own shape: {"error": {code, message}, "request_id": …}
        assert!(v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown resource"));
        assert_eq!(v["error"]["code"].as_str(), Some("invalid_request"));
        assert_eq!(v["request_id"].as_str(), Some("corr-1"));
    }

    #[test]
    fn re_exports_are_the_same_types_the_api_uses() {
        // compile-time pin: a Page built here IS the api::Page type
        let p: Page = Page {
            limit: 10,
            offset: 0,
        };
        assert_eq!(p.limit, 10);
        let st = Status::parse("ok").unwrap();
        assert_eq!(st.as_str(), "ok");
    }
}
