//! Request execution abstraction: the typed envelope a crawl job wears on
//! its way through the fetcher — plan (what to ask for), context (who/why,
//! correlation), outcome metadata (what it cost).
//!
//! The fetcher stays the single owner of transport rules (timeouts, body
//! caps, redirects, retry policy). This module makes the *decision inputs
//! and observability outputs* explicit so the engine, events, and admin
//! snapshots all speak about the same request without re-deriving strings.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use url::Url;

use crate::accounting::Accounting;
use crate::events::{Bus, EventKind};
use crate::job::JobId;
use crate::retry::RetryBudget;

/// What we intend to do: the request itself plus hard local limits.
/// Deadlines here are *absolute*; the retry loop refuses to schedule an
/// attempt that cannot start in time instead of sleeping optimistically.
#[derive(Debug, Clone)]
pub struct RequestPlan {
    pub url: Url,
    /// Extra headers for conditional GET (If-None-Match / If-Modified-Since).
    pub headers: Vec<(String, String)>,
    /// Run-wide response cap mirrored here so a plan can be validated or
    /// unit-tested independently of config.
    pub max_response_bytes: u64,
    /// When the whole exchange must be over; None = only the transport
    /// timeouts apply.
    pub deadline: Option<Instant>,
}

impl RequestPlan {
    pub fn new(url: Url) -> Self {
        Self {
            url,
            headers: Vec::new(),
            max_response_bytes: 0,
            deadline: None,
        }
    }
    pub fn with_headers(mut self, h: Vec<(String, String)>) -> Self {
        self.headers = h;
        self
    }
    pub fn with_deadline(mut self, d: Instant) -> Self {
        self.deadline = Some(d);
        self
    }
    pub fn expired(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|d| now >= d)
    }
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(now))
    }
}

/// Who is asking, for which job, and where to report. One per request
/// attempt-chain; cheap to build per job (no allocation when bus is None).
#[derive(Debug, Clone, Default)]
pub struct RequestContext {
    /// Correlation id shared by every event line of this exchange chain —
    /// the join key between engine internals and an operator's grep.
    pub correlation_id: u64,
    pub job_id: Option<JobId>,
    pub target: usize,
    /// Host for log/event grouping (no credentials ever go in here).
    pub host: String,
    /// Observation sink; None on internal fetches that don't publish
    /// (robots/sitemap keep a quiet stream unless a run bus is provided).
    /// Not part of any wire format — this struct is process-local.
    pub bus: Option<Arc<Bus>>,
    /// Run accounting; None = this fetch doesn't move counters (tests,
    /// admin-triggered robots pings).
    pub acct: Option<Arc<Accounting>>,
    /// Shared per-minute retry budget; None = unlimited (default).
    pub budget: Option<Arc<RetryBudget>>,
}

impl RequestContext {
    pub fn publish(&self, kind: EventKind, detail: serde_json::Value) {
        if let Some(bus) = &self.bus {
            bus.publish(kind, self.job_id.map(|j| j.0), detail);
        }
    }
    pub fn acct_note_bytes(&self, n: u64) {
        if let Some(a) = &self.acct {
            a.note_bytes(n);
        }
    }
    pub fn acct_note_retry(&self, delay_ms: u64) {
        if let Some(a) = &self.acct {
            a.note_retry(delay_ms);
        }
    }
}

/// Per-exchange timing/result metadata, produced by the fetcher and read by
/// engine accounting + events.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ExecutionMeta {
    pub correlation_id: u64,
    pub attempts: u32,
    pub total_ms: u64,
    pub last_attempt_ms: u64,
    pub bytes: u64,
    pub status: u16,
    pub egress: String,
    /// Sum of backoff sleeps inside this exchange (retry policy accounting).
    pub backoff_ms: u64,
    /// Whether we gave up due to a local deadline rather than the server.
    pub deadline_hit: bool,
}

#[cfg(test)]
mod tests;
