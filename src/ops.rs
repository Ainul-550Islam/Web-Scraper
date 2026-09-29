//! Operator-plane state: bounded rings that record what happened *around*
//! the crawl (run lifecycle markers seen by operators, alert
//! acknowledgements, event-log toggles) — kept separate from the engine
//! event bus so an operator action never competes with crawl traffic for
//! ring space, and so `GET /ops` / `GET /audit` can serve from a fixed-size
//! structure without touching the filesystem.
//!
//! Both rings are append-evict (`VecDeque` with a hard cap): memory is O(cap)
//! forever, snapshot order is oldest-first, and `tail(n)` is newest-first
//! after a single reversal at the caller.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;

use crate::api::epoch_secs;
use crate::events::EventKind;
use crate::runtime::RuntimeSlot;

/// Maximum retained operator events / audit records. A daemon running for
/// months still only holds these many rows — the point is that the newest
/// history is always on hand and memory never grows.
pub const OPS_CAP: usize = 256;
pub const AUDIT_CAP: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpsEvent {
    pub ts: u64,
    /// Bounded, lowercase, snake_case — the same vocabulary as bus kinds.
    pub kind: String,
    /// Small JSON facts (counts, statuses). Sanitized at the producer; no
    /// secrets ride here.
    pub detail: Value,
}

/// One operator action against the control plane. `actor` is coarse
/// ("admin-api"/"cli"), `resource` is a *safe* identifier (an event name,
/// a target id — never a token, never a full path), `corr` links the action
/// to the access log line.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditRecord {
    pub ts: u64,
    pub actor: String,
    pub action: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corr: Option<String>,
    /// Small, producer-sanitized JSON facts: which provider a job named,
    /// whether external processing was consented to, which limits were asked
    /// for. Never a payload, never a path, never a credential.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// Clip to `limit` chars and replace anything that could forge log lines
/// (control bytes, DEL) with '?'. Used for every caller-supplied string
/// stored in the rings.
pub fn sanitize(s: &str, limit: usize) -> String {
    s.chars()
        .map(|c| {
            if (c as u32) < 0x20 || c as u32 == 0x7f {
                '?'
            } else {
                c
            }
        })
        .take(limit)
        .collect()
}

#[derive(Debug, Default)]
pub struct OpsLog {
    ring: Mutex<VecDeque<OpsEvent>>,
}

impl OpsLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record into the ring and, when a runtime is attached, mirror onto the
    /// engine event bus as `operator_action` (so live tailing sees it too).
    pub fn record(&self, runtime: &RuntimeSlot, kind: &str, detail: Value) {
        let ev = OpsEvent {
            ts: epoch_secs(),
            kind: sanitize(kind, 64),
            detail,
        };
        {
            let mut g = self.ring.lock().unwrap_or_else(|p| p.into_inner());
            if g.len() >= OPS_CAP {
                g.pop_front();
            }
            g.push_back(ev.clone());
        }
        if let Some(rt) = runtime.get() {
            rt.emit(
                EventKind::OperatorAction,
                None,
                serde_json::json!({ "kind": ev.kind, "detail": ev.detail }),
            );
        }
    }

    /// Oldest-first, all that is retained.
    pub fn snapshot(&self) -> Vec<OpsEvent> {
        self.ring
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Newest-first, at most `n`.
    pub fn tail(&self, n: usize) -> Vec<OpsEvent> {
        let g = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        g.iter().rev().take(n).cloned().collect()
    }
}

#[derive(Debug, Default)]
pub struct AuditLedger {
    ring: Mutex<VecDeque<AuditRecord>>,
}

impl AuditLedger {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn note(
        &self,
        actor: &str,
        action: &str,
        result: &str,
        resource: Option<&str>,
        corr: &str,
    ) {
        self.note_with_detail(actor, action, result, resource, corr, None);
    }

    /// Same as [`Self::note`], with a small JSON fact sheet attached. The
    /// detail is clipped so one noisy caller cannot bloat the ring; callers
    /// are expected to pass identifiers and booleans, never content.
    pub fn note_with_detail(
        &self,
        actor: &str,
        action: &str,
        result: &str,
        resource: Option<&str>,
        corr: &str,
        detail: Option<Value>,
    ) {
        let rec = AuditRecord {
            ts: epoch_secs(),
            actor: sanitize(actor, 32),
            action: sanitize(action, 64),
            result: sanitize(result, 32),
            resource: resource.map(|r| sanitize(r, 128)),
            corr: Some(sanitize(corr, 64)),
            detail: detail.map(|d| {
                let s = d.to_string();
                match serde_json::from_str::<Value>(&sanitize(&s, 512)) {
                    Ok(v) => v,
                    Err(_) => Value::Null,
                }
            }),
        };
        let mut g = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() >= AUDIT_CAP {
            g.pop_front();
        }
        g.push_back(rec);
    }

    pub fn snapshot(&self) -> Vec<AuditRecord> {
        self.ring
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Newest-first, at most `n`.
    pub fn tail(&self, n: usize) -> Vec<AuditRecord> {
        let g = self.ring.lock().unwrap_or_else(|p| p.into_inner());
        g.iter().rev().take(n).cloned().collect()
    }
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests;
