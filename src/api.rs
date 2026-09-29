//! Typed API domain models, query parsing, and the error surface shared by
//! the admin HTTP API (`src/admin.rs`) and the CLI (`src/main.rs`).
//!
//! Everything here is data + validation: no serving, no I/O besides the tiny
//! loopback HTTP client the CLI uses to talk to a running `harvest serve`.
//! The models deliberately *reuse* the live types where they already exist
//! (`crate::proxy::ProxySnapshot`, `crate::sched::QueueStats`,
//! `crate::supervisor::SupervisorSnapshot`, `crate::accounting` snapshots)
//! so there is exactly one source of truth for every number.
//!
//! # Guarantees
//!
//! * Every list/query accepts bounded pagination (`limit` is clamped, never
//!   unbounded); sorting keys are validated against a small whitelist.
//! * [`ApiError`] is the only error shape the API renders. It carries a
//!   stable machine code, an operator-facing message, and the HTTP status —
//!   never a stack, path, or secret.
//! * Correlation ids are echoed when well-formed and generated otherwise,
//!   and every response carries them (see [`CorrelationId`]).

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use crate::config::Config;

pub(crate) fn epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ------------------------------------------------------------------- errors

/// A single, honest error shape for every non-2xx response. `message` is
/// written for operators: it may name a parameter and its bound, but never
/// echoes secret values, filesystem paths from config, or internal types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

impl ApiError {
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: 401,
            code: "unauthorized",
            message: msg.into(),
        }
    }
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self {
            status: 403,
            code: "forbidden",
            message: msg.into(),
        }
    }
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "invalid_request",
            message: msg.into(),
        }
    }
    /// 409: the request is well-formed but conflicts with current state
    /// (cancelling a finished job, reusing an idempotency key, downloading an
    /// artifact that is not finalized).
    pub fn conflict(msg: impl Into<String>) -> Self {
        Self {
            status: 409,
            code: "conflict",
            message: msg.into(),
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: 404,
            code: "not_found",
            message: msg.into(),
        }
    }
    pub fn method_not_allowed(method: &str, path: &str) -> Self {
        Self {
            status: 405,
            code: "method_not_allowed",
            message: format!("{method} is not supported on {path}; this API is read-mostly (only POST /alerts/ack and POST /ops/note mutate)"),
        }
    }
    pub fn payload_too_large(limit: u64) -> Self {
        Self {
            status: 413,
            code: "payload_too_large",
            message: format!("request body exceeds the {limit} byte limit"),
        }
    }
    pub fn unavailable(msg: impl Into<String>) -> Self {
        Self {
            status: 503,
            code: "unavailable",
            message: msg.into(),
        }
    }
    /// 500 keeps the cause in the log (`tracing`), not in the response.
    pub fn internal() -> Self {
        Self {
            status: 500,
            code: "internal",
            message: "internal error; see server log (correlation id)".into(),
        }
    }

    /// Stable JSON envelope: `{"error":{code,message}}`, key order fixed.
    pub fn to_json(&self, corr: &str) -> Value {
        json!({
            "error": { "code": self.code, "message": self.message },
            "request_id": corr,
        })
    }

    pub fn status_line(&self) -> &'static str {
        match self.status {
            400 => "400 Bad Request",
            401 => "401 Unauthorized",
            403 => "403 Forbidden",
            404 => "404 Not Found",
            405 => "405 Method Not Allowed",
            409 => "409 Conflict",
            413 => "413 Payload Too Large",
            500 => "500 Internal Server Error",
            _ => "503 Service Unavailable",
        }
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.code)
    }
}

impl std::error::Error for ApiError {}

// ------------------------------------------------------------ correlation

/// Re-exported from [`crate::ids`] so the correlation rule lives in exactly
/// one place; the wire shape and validation are unchanged.
pub use crate::ids::CorrelationId;

// ------------------------------------------------------------------ query

/// Percent-decoded, bounded query-string view. Hard caps: 2 KiB input, 32
/// pairs, 256-byte values — anything bigger is a parse error the caller
/// maps to 400, so no handler downstream can be fed an unbounded param.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pairs: BTreeMap<String, String>,
}

impl Query {
    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        if raw.len() > 2048 {
            return Err(ApiError::invalid("query string exceeds 2048 bytes"));
        }
        let mut pairs = BTreeMap::new();
        let mut seen = 0usize;
        for part in raw.split('&').filter(|p| !p.is_empty()) {
            seen += 1;
            if seen > 32 {
                return Err(ApiError::invalid("too many query parameters (max 32)"));
            }
            let (k, v) = match part.split_once('=') {
                Some((k, v)) => (k, v),
                None => (part, ""),
            };
            let k = decode_component(k)?;
            let v = decode_component(v)?;
            if v.len() > 256 || k.len() > 64 {
                return Err(ApiError::invalid("query parameter too long"));
            }
            // Last occurrence wins: deterministic, documented, matches most
            // HTTP frameworks (repeated keys are a client bug, not a fork).
            pairs.insert(k, v);
        }
        Ok(Self { pairs })
    }

    pub fn get(&self, k: &str) -> Option<&str> {
        self.pairs.get(k).map(String::as_str)
    }

    /// `?k` present at all (even empty) — used for flags like `?prom`.
    pub fn has(&self, k: &str) -> bool {
        self.pairs.contains_key(k)
    }

    pub fn parse_u64(&self, k: &str) -> Result<Option<u64>, ApiError> {
        match self.get(k) {
            None | Some("") => Ok(None),
            Some(v) => v
                .parse::<u64>()
                .map(Some)
                .map_err(|_| ApiError::invalid(format!("{k} must be a non-negative integer"))),
        }
    }
}

fn decode_component(s: &str) -> Result<String, ApiError> {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hi = *b
                    .get(i + 1)
                    .ok_or_else(|| ApiError::invalid("truncated % escape"))?;
                let lo = *b
                    .get(i + 2)
                    .ok_or_else(|| ApiError::invalid("truncated % escape"))?;
                let hv = (hi as char)
                    .to_digit(16)
                    .ok_or_else(|| ApiError::invalid("bad % escape"))?;
                let lv = (lo as char)
                    .to_digit(16)
                    .ok_or_else(|| ApiError::invalid("bad % escape"))?;
                let b = hv as u8 * 16 + lv as u8;
                if b < 0x20 || b == 0x7f {
                    return Err(ApiError::invalid("control byte in query parameter"));
                }
                out.push(b as char);
                i += 3;
            }
            b'+' => {
                out.push(' ');
                i += 1;
            }
            // Only ASCII-safe passthrough bytes; control chars and quotes
            // never make it through, so values cannot forge log lines.
            c if c < 0x21 || matches!(c, b'"' | b'\'' | b'\\') => {
                return Err(ApiError::invalid("illegal byte in query parameter"));
            }
            c => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Bounded pagination shared by every list endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Page {
    pub limit: usize,
    pub offset: usize,
}

pub const PAGE_DEFAULT_LIMIT: usize = 50;
pub const PAGE_MAX_LIMIT: usize = 200;
pub const PAGE_MAX_OFFSET: u64 = 1_000_000;

impl Page {
    pub fn from_query(q: &Query) -> Result<Self, ApiError> {
        let limit = match q.parse_u64("limit")? {
            Some(0) => return Err(ApiError::invalid("limit must be >= 1")),
            Some(v) => (v as usize).min(PAGE_MAX_LIMIT),
            None => PAGE_DEFAULT_LIMIT,
        };
        let offset = q.parse_u64("offset")?.unwrap_or(0);
        if offset > PAGE_MAX_OFFSET {
            return Err(ApiError::invalid("offset too large"));
        }
        Ok(Self {
            limit,
            offset: offset as usize,
        })
    }

    /// Apply page to `items` and report the total before slicing.
    pub fn window<T>(&self, items: Vec<T>) -> PageWindow<T> {
        let total = items.len();
        let slice = items
            .into_iter()
            .skip(self.offset)
            .take(self.limit)
            .collect();
        PageWindow {
            total,
            limit: self.limit,
            offset: self.offset,
            items: slice,
        }
    }
}

/// The uniform envelope for every list response.
#[derive(Debug, Clone, Serialize)]
pub struct PageWindow<T> {
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
    pub items: Vec<T>,
}

// ----------------------------------------------------------------- models

/// Lifecycle of the collection job this process is running (or drained).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Idle,
    Running,
    Draining,
}

/// Per-target live counters — the typed projection of the engine's
/// per-target tally as it appears in run summaries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobInfo {
    pub target: String,
    pub written: u64,
    pub blocked: u64,
    pub failed: u64,
    pub revalidated: u64,
    pub quarantined: u64,
}

impl JobInfo {
    pub fn pages(&self) -> u64 {
        self.written + self.blocked + self.failed + self.revalidated + self.quarantined
    }
}

/// Live job view: engine status + one row per configured target.
#[derive(Debug, Clone, Serialize)]
pub struct JobSummary {
    pub run: String,
    pub status: JobStatus,
    pub uptime_ms: u64,
    pub jobs: Vec<JobInfo>,
}

/// A row of the run-history JSONL (`schedule.history_path`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunInfo {
    /// 0-based position in the history file (stable tiebreak for sorts).
    pub index: u64,
    pub ts: u64,
    pub run: String,
    pub status: String,
    pub duration_ms: u64,
    pub config_sha256: Option<String>,
    /// Written records in that run (from the embedded summary).
    pub written: u64,
    pub quarantined: u64,
    pub error: Option<String>,
}

impl RunInfo {
    /// Extract from one parsed history line. Missing/odd types degrade to
    /// zero/absent rather than failing the whole listing.
    pub fn from_history(v: &Value, index: u64) -> RunInfo {
        let g = |k: &str| v.get(k).and_then(Value::as_u64);
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let sum = v.get("summary");
        RunInfo {
            index,
            ts: g("ts").unwrap_or(0),
            run: s("run"),
            status: s("status"),
            duration_ms: g("duration_ms").unwrap_or(0),
            config_sha256: v
                .get("config_sha256")
                .and_then(Value::as_str)
                .map(str::to_string),
            written: sum
                .and_then(|x| x.get("written"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            quarantined: sum
                .and_then(|x| x.get("quarantined"))
                .and_then(Value::as_u64)
                .unwrap_or(0),
            error: v.get("error").and_then(Value::as_str).map(str::to_string),
        }
    }
}

/// Sort spec: whitelisted key + direction; anything else is a 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Ts,
    Duration,
    Written,
    Name,
    Bytes,
}

impl SortKey {
    pub fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "ts" => Ok(Self::Ts),
            "duration" => Ok(Self::Duration),
            "written" => Ok(Self::Written),
            "name" => Ok(Self::Name),
            "bytes" => Ok(Self::Bytes),
            other => Err(ApiError::invalid(format!(
                "unknown sort key `{other}` (allowed: ts, duration, written, name, bytes)"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SortDir {
    Asc,
    Desc,
}

/// Parsed list params: `sort=<key>&order=asc|desc` plus free-form equality
/// filters (`status=...`) and `since`/`until` epoch-second ranges.
#[derive(Debug, Clone, Copy)]
pub struct ListFilter {
    pub sort: Option<(SortKey, SortDir)>,
    pub status: Option<Status>,
    pub since: Option<u64>,
    pub until: Option<u64>,
}

/// Small `status` vocabulary shared by runs listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok,
    Error,
    Cancelled,
}

impl Status {
    pub fn parse(s: &str) -> Result<Self, ApiError> {
        match s {
            "ok" => Ok(Self::Ok),
            "error" => Ok(Self::Error),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(ApiError::invalid(format!(
                "unknown status `{other}` (allowed: ok, error, cancelled)"
            ))),
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

impl ListFilter {
    pub fn from_query(q: &Query) -> Result<Self, ApiError> {
        let sort = match q.get("sort") {
            None | Some("") => None,
            Some(s) => Some((
                SortKey::parse(s)?,
                match q.get("order").unwrap_or("desc") {
                    "asc" => SortDir::Asc,
                    "desc" => SortDir::Desc,
                    other => {
                        return Err(ApiError::invalid(format!(
                            "order must be asc or desc (got `{other}`)"
                        )))
                    }
                },
            )),
        };
        let status = match q.get("status") {
            None | Some("") => None,
            Some(s) => Some(Status::parse(s)?),
        };
        let since = q.parse_u64("since")?;
        let until = q.parse_u64("until")?;
        if let (Some(a), Some(b)) = (since, until) {
            if a > b {
                return Err(ApiError::invalid("since must be <= until"));
            }
        }
        Ok(Self {
            sort,
            status,
            since,
            until,
        })
    }

    pub fn keep_run(&self, r: &RunInfo) -> bool {
        if let Some(s) = self.status {
            if r.status != s.as_str() {
                return false;
            }
        }
        if let Some(x) = self.since {
            if r.ts < x {
                return false;
            }
        }
        if let Some(x) = self.until {
            if r.ts > x {
                return false;
            }
        }
        true
    }

    /// Deterministic total order: requested key, tie-broken by history
    /// index — so two clients paging over the same file see identical pages.
    pub fn sort_runs(&self, items: &mut [RunInfo]) {
        let (key, dir) = match self.sort {
            Some(k) => k,
            None => (SortKey::Ts, SortDir::Desc),
        };
        items.sort_by(|a, b| {
            let ord = match key {
                SortKey::Ts => a.ts.cmp(&b.ts),
                SortKey::Duration => a.duration_ms.cmp(&b.duration_ms),
                SortKey::Written => a.written.cmp(&b.written),
                _ => std::cmp::Ordering::Equal,
            };
            let ord = match ord {
                std::cmp::Ordering::Equal => a.index.cmp(&b.index),
                o => o,
            };
            match dir {
                SortDir::Asc => ord,
                SortDir::Desc => ord.reverse(),
            }
        });
    }
}

// ------------------------------------------------------ live-state models

/// Uniform internal component health. Note the four states: `unavailable`
/// (configured but not reachable/attached) is deliberately distinct from
/// `disabled` (not configured at all) — an operator greps for the former.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentStatus {
    Healthy,
    Degraded,
    Unavailable,
    Disabled,
}

impl ComponentStatus {
    pub fn rank(self) -> u8 {
        match self {
            Self::Healthy | Self::Disabled => 0,
            Self::Degraded => 1,
            Self::Unavailable => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ComponentReport {
    pub name: &'static str,
    pub status: ComponentStatus,
    /// Small, bounded facts (counts, ms) — never paths, URLs, or tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

/// `/diagnostics` payload. The public `/health` probe stays the tiny
/// `{status, running}` body it has always had; this is the extended,
/// authenticated view used by `harvest health` and incident tooling.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HealthReport {
    pub overall: ComponentStatus,
    pub generated_ts: u64,
    pub components: Vec<ComponentReport>,
}

impl HealthReport {
    pub fn build(mut components: Vec<ComponentReport>) -> Self {
        let overall = components
            .iter()
            .map(|c| c.status)
            .max_by_key(|s| s.rank())
            .unwrap_or(ComponentStatus::Healthy);
        components.sort_by(|a, b| a.name.cmp(b.name));
        Self {
            overall: if overall == ComponentStatus::Disabled {
                ComponentStatus::Healthy
            } else {
                overall
            },
            generated_ts: epoch_secs(),
            components,
        }
    }
}

/// Queue view. `stats` is the live `sched::QueueStats` serialization passed
/// through unchanged (one source of truth, no field-by-field copy), wrapped
/// with an explicit lifecycle state so `/queue` answers 200 with
/// `disabled|unavailable` instead of a bare 503 (scripting-friendly).
#[derive(Debug, Clone, Serialize)]
pub struct QueueHealth {
    pub state: ComponentStatus,
    pub stats: Option<Value>,
    /// Mean submit->start wait over finished jobs (accounting source).
    pub avg_queue_wait_ms: f64,
}

/// Worker-pool view: the supervisor `SupervisorSnapshot` passed through with
/// a lifecycle state attached (see `QueueHealth` for why).
#[derive(Debug, Clone, Serialize)]
pub struct WorkerHealth {
    pub state: ComponentStatus,
    pub snapshot: Option<Value>,
}

impl QueueHealth {
    /// `snap`: the runtime snapshot JSON (queue may be `null` when no run
    /// has attached a queue yet).
    pub fn from_snapshot(snap: &Value, running: bool) -> Self {
        match snap.get("queue") {
            Some(v) if !v.is_null() => Self {
                state: if running {
                    ComponentStatus::Healthy
                } else {
                    ComponentStatus::Degraded
                },
                avg_queue_wait_ms: snap
                    .pointer("/accounting/avg_queue_wait_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                stats: Some(v.clone()),
            },
            _ => Self {
                state: ComponentStatus::Unavailable,
                stats: None,
                avg_queue_wait_ms: 0.0,
            },
        }
    }
}

impl WorkerHealth {
    pub fn from_snapshot(snap: &Value, running: bool) -> Self {
        match snap.get("workers") {
            Some(v) if !v.is_null() => {
                // deaths>0 is the one real failure signal the supervisor
                // exposes (a task died and had to be restarted); idle
                // between jobs is normal, not degraded.
                let deaths = v.get("deaths").and_then(Value::as_u64).unwrap_or(0);
                let _ = running;
                Self {
                    state: if deaths > 0 {
                        ComponentStatus::Degraded
                    } else {
                        ComponentStatus::Healthy
                    },
                    snapshot: Some(v.clone()),
                }
            }
            _ => Self {
                state: ComponentStatus::Unavailable,
                snapshot: None,
            },
        }
    }
}

/// Proxy pool with a bounded, derived health classification per egress.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyPoolInfo {
    pub total: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub down: usize,
    pub unprobed: usize,
    pub entries: Vec<ProxyEgressInfo>,
}

/// One egress row: exactly the pool's public snapshot fields (which never
/// include the proxy URL) plus the classification.
#[derive(Debug, Clone, Serialize)]
pub struct ProxyEgressInfo {
    #[serde(flatten)]
    pub snap: crate::proxy::ProxySnapshot,
    pub state: &'static str,
}

impl ProxyEgressInfo {
    pub fn classify(snap: &crate::proxy::ProxySnapshot) -> Self {
        let state = if snap.requests == 0 {
            "unprobed"
        } else if snap.success_ewma >= 0.9 {
            "healthy"
        } else if snap.success_ewma >= 0.5 {
            "degraded"
        } else {
            "down"
        };
        Self {
            snap: snap.clone(),
            state,
        }
    }

    pub fn pool_info(snapshots: Vec<crate::proxy::ProxySnapshot>) -> ProxyPoolInfo {
        let entries: Vec<_> = snapshots.iter().map(Self::classify).collect();
        let count = |s: &str| entries.iter().filter(|e| e.state == s).count();
        ProxyPoolInfo {
            total: entries.len(),
            healthy: count("healthy"),
            degraded: count("degraded"),
            down: count("down"),
            unprobed: count("unprobed"),
            entries,
        }
    }
}

/// Pipeline view: counters from `Metrics`, structure from `Config`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineStats {
    pub written: u64,
    pub quarantined: u64,
    pub schema_drifts: u64,
    pub pipeline_failures: u64,
    pub per_target: Vec<PipelineTargetStats>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineTargetStats {
    pub target: String,
    pub transform_ops: usize,
    pub schema_fields: usize,
    pub enforce: String,
}

impl PipelineStats {
    pub fn from(cfg: &Config, metrics: &crate::metrics::Metrics) -> Self {
        let j = metrics.display_json();
        let per_target = cfg
            .targets
            .iter()
            .map(|t| PipelineTargetStats {
                target: t.id.clone(),
                transform_ops: t.extract.values().map(|f| f.transforms.len()).sum(),
                schema_fields: t.schema.fields.len(),
                enforce: t.schema.enforce.clone(),
            })
            .collect();
        Self {
            written: j["records_written"].as_u64().unwrap_or(0),
            quarantined: j["records_quarantined"].as_u64().unwrap_or(0),
            schema_drifts: j["schema_drifts"].as_u64().unwrap_or(0),
            pipeline_failures: j["pipeline_failures"].as_u64().unwrap_or(0),
            per_target,
        }
    }
}

/// Storage lane view: file-backed health of every configured lane. Derived
/// from `Config` + `fs::metadata` — it reports what is on disk right now,
/// it never probes databases (that stays in the per-feature live tests).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StorageHealth {
    pub output: LaneHealth,
    pub history: LaneHealth,
    pub events: LaneHealth,
    pub audit: LaneHealth,
    pub quarantine: LaneHealth,
    pub checkpoint: LaneHealth,
    pub revalidation: LaneHealth,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LaneHealth {
    /// Whether the lane is configured at all.
    pub configured: bool,
    /// True when the path/`kind` points at something that currently exists.
    pub exists: bool,
    /// Bytes on disk (files/dirs summed); `None` when not applicable.
    pub bytes: Option<u64>,
    /// Path *label*, never the raw path (see `path_label`).
    pub path: String,
    /// Human note: "disabled", "directory", "sqlite file", ...
    pub kind: &'static str,
}

/// Redacted path display: basename plus one parent component. Private
/// filesystem layout must not leak wholesale, but an operator must be able
/// to tell `dataset.p1.jsonl` from `history.jsonl`.
pub fn path_label(p: &std::path::Path) -> String {
    let name = p
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".into());
    match p.parent().and_then(|d| d.file_name()) {
        Some(parent) => {
            let leaf = if name == "?" {
                String::new()
            } else {
                format!("/{name}")
            };
            format!("…/{}{leaf}", parent.to_string_lossy())
        }
        None => name,
    }
}

fn file_lane(p: Option<&std::path::Path>, kind: &'static str) -> LaneHealth {
    match p {
        None => LaneHealth {
            configured: false,
            exists: false,
            bytes: None,
            path: String::new(),
            kind: if kind == "disabled" { "disabled" } else { kind },
        },
        Some(path) => {
            let meta = std::fs::metadata(path).ok();
            let bytes = meta.as_ref().map(|m| {
                if m.is_dir() {
                    dir_size_bounded(path, 256)
                } else {
                    m.len()
                }
            });
            LaneHealth {
                configured: true,
                exists: meta.is_some(),
                bytes,
                path: path_label(path),
                kind,
            }
        }
    }
}

/// Non-recursive dir size over at most `cap` entries (datasets can hold
/// unbounded part files; the scan must not).
pub fn dir_size_bounded(dir: &std::path::Path, cap: usize) -> u64 {
    let mut total = 0u64;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten().take(cap) {
            if let Ok(m) = e.metadata() {
                if m.is_file() {
                    total += m.len();
                }
            }
        }
    }
    total
}

impl StorageHealth {
    pub fn from(cfg: &Config) -> Self {
        use crate::config::SinkCfg;
        let (out_kind, out_path) = match &cfg.output {
            SinkCfg::Dataset { dir, .. } => ("dataset dir", Some(dir.clone())),
            SinkCfg::Jsonl { path } | SinkCfg::Csv { path } => {
                ("jsonl/csv file", Some(path.clone()))
            }
            SinkCfg::Sqlite { path } => ("sqlite file", Some(path.clone())),
            _ => ("remote or none", None),
        };
        Self {
            output: file_lane(out_path.as_deref(), out_kind).clone(),
            history: file_lane(Some(&cfg.schedule.history_path), "history jsonl"),
            events: file_lane(cfg.events_path.as_deref(), "event log jsonl"),
            audit: file_lane(cfg.audit_path.as_deref(), "audit json"),
            // per-target lanes; the first configured one is the representative
            // (the multi-target total is `records_quarantined` on /stats).
            quarantine: file_lane(
                cfg.targets
                    .iter()
                    .find_map(|t| t.schema.quarantine_path.as_deref()),
                "quarantine jsonl",
            ),
            checkpoint: file_lane(cfg.ckpt_path.as_deref(), "checkpoint json"),
            revalidation: file_lane(cfg.revalidate_store.as_deref(), "revalidation store"),
        }
    }
}

/// Bounded read of the newest lines of the history log: the tail of the last
/// 256 KiB (never the whole file - a months-long `serve` history is huge and
/// this endpoint must stay cheap on a scrape loop). Garbage lines are
/// dropped, not fatal. Index is position-in-window, so sort ties preserve
/// file order for a given (immutable, append-only) file.
const HISTORY_WINDOW_BYTES: u64 = 256 * 1024;
pub const HISTORY_WINDOW_LINES: usize = 512;

pub fn history_window(path: &std::path::Path, max_lines: usize) -> Vec<Value> {
    let max_lines = max_lines.clamp(1, HISTORY_WINDOW_LINES);
    let txt = match std::fs::metadata(path) {
        Ok(m) if m.len() == 0 => return Vec::new(),
        Ok(m) => {
            use std::io::{Read, Seek, SeekFrom};
            let len = m.len().min(HISTORY_WINDOW_BYTES);
            let mut f = match std::fs::File::open(path) {
                Ok(f) => f,
                Err(_) => return Vec::new(),
            };
            if m.len() > len && f.seek(SeekFrom::End(-(len as i64))).is_err() {
                return Vec::new();
            }
            let mut raw = Vec::with_capacity(len as usize);
            if f.read_to_end(&mut raw).is_err() {
                return Vec::new();
            }
            // If we started mid-file, the first line is partial: drop it.
            let start = if m.len() > len {
                match raw.iter().position(|b| *b == b'\n') {
                    Some(p) => p + 1,
                    None => raw.len(),
                }
            } else {
                0
            };
            String::from_utf8_lossy(&raw[start..]).into_owned()
        }
        Err(_) => return Vec::new(),
    };
    let lines: Vec<&str> = txt.lines().collect();
    let skip = lines.len().saturating_sub(max_lines);
    lines[skip..]
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect()
}

/// Newest `limit` valid history lines as `RunInfo` rows (file order).
pub fn recent_runs(path: &std::path::Path, limit: usize) -> Vec<RunInfo> {
    history_window(path, limit)
        .into_iter()
        .enumerate()
        .map(|(i, v)| RunInfo::from_history(&v, i as u64))
        .collect()
}

/// Full `/runs`-style listing shared by the admin route and the CLI:
/// filter, deterministic sort, bounded page. Returns (window_total, page).
pub fn runs_listing(
    path: &std::path::Path,
    filter: &ListFilter,
    page: Page,
) -> (usize, PageWindow<RunInfo>) {
    let window = history_window(path, HISTORY_WINDOW_LINES);
    let total = window.len();
    let mut runs: Vec<RunInfo> = window
        .iter()
        .enumerate()
        .map(|(i, v)| RunInfo::from_history(v, i as u64))
        .filter(|r| filter.keep_run(r))
        .collect();
    filter.sort_runs(&mut runs);
    (total, page.window(runs))
}

// ------------------------------------------------------- config inspect

/// Redaction key set: any JSON object key whose lowercase name is here has
/// its string value replaced. Secrets never appear in any diagnostic output.
const SECRET_KEYS: &[&str] = &[
    "admin_token",
    "attest_key_path",
    "token",
    "password",
    "secret",
    "dsn",
    "webhook_url",
    "redis_url",
];

/// True for the small set of keys where a URL *is* the secret (it carries
/// credentials); rendered as origin-only.
const URL_KEYS: &[&str] = &["webhook_url", "redis_url", "dsn"];

/// Recursively redact + annotate a config value tree. Returns the redacted
/// value with a parallel `_source` map: per top-level key, `default` |
/// `explicit` | `env:NAME` (raw whole-value `${VAR}` in the file), so the
/// operator can see *where* each setting came from without us storing any
/// extra provenance state.
pub fn redact_config(loaded: &mut Value, raw: Option<&Value>) {
    let Some(obj) = loaded.as_object_mut() else {
        return;
    };
    // Provenance annotation at the top level only (that is where defaults
    // and env refs are decided by `Config::default` / load expansion). It
    // must run BEFORE redaction mutates values, or redaction artifacts
    // would look like differences from the defaults.
    let defaults = serde_json::to_value(Config::default())
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let mut sources = Map::new();
    for (k, v) in obj.iter() {
        let source = match raw.and_then(|r| r.get(k)) {
            None => {
                if defaults.get(k) == Some(v) {
                    "default"
                } else {
                    // derived (e.g. source_hash) or default with env effect
                    "derived"
                }
            }
            Some(r) => match r.as_str().and_then(|s| s.strip_prefix("${")) {
                Some(rest) if rest.ends_with('}') => {
                    let var = &rest[..rest.len() - 1];
                    if !var.is_empty()
                        && var.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    {
                        sources.insert(k.clone(), json!(format!("env:{var}")));
                        continue;
                    }
                    "explicit"
                }
                _ => "explicit",
            },
        };
        sources.insert(k.clone(), json!(source));
    }
    redact_walk(obj);
    obj.insert("_sources".into(), Value::Object(sources));
}

fn redact_walk(map: &mut Map<String, Value>) {
    for (k, v) in map.iter_mut() {
        match v {
            Value::String(s) => {
                let key = k.to_ascii_lowercase();
                if SECRET_KEYS.contains(&key.as_str()) {
                    if URL_KEYS.contains(&key.as_str()) {
                        *s = format!("{}<redacted>", crate::config::redact_url_origin(s));
                    } else {
                        *s = "<redacted>".into();
                    }
                } else if s.contains("://") {
                    // Credential-bearing URLs under non-secret key names
                    // (proxy `url`s are the standing example) are scrubbed;
                    // plain enum-like strings pass through untouched.
                    *s = crate::config::redact_credentials(s);
                }
            }
            Value::Object(inner) => redact_walk(inner),
            Value::Array(items) => {
                for it in items.iter_mut() {
                    if let Value::Object(inner) = it {
                        redact_walk(inner);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Full `/config` payload: redacted effective config + validation status +
/// backend availability for this binary. `path` (when known) adds provenance
/// by re-reading the raw file and diffing against the expanded value.
pub fn config_inspect(cfg: &Config, path: Option<&std::path::Path>) -> Value {
    let mut v = serde_json::to_value(cfg).unwrap_or_else(|_| json!({}));
    let raw = path.and_then(|p| {
        std::fs::read_to_string(p)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
    });
    redact_config(&mut v, raw.as_ref());
    if let Value::Object(obj) = &mut v {
        // Load-time soft warnings, feature/backend availability, and the
        // production audit ride alongside the redacted document — every
        // ops surface (`harvest config inspect`, `GET /config`) gets them
        // from the one builder.
        obj.insert(
            "_warnings".into(),
            Value::Array(cfg.load_warnings.iter().cloned().map(Value::from).collect()),
        );
        obj.insert(
            "_backends".into(),
            json!({
                "sqlite": cfg!(feature = "sqlite"),
                "postgres": cfg!(feature = "postgres"),
                "redis": cfg!(feature = "redis"),
            }),
        );
        obj.insert(
            "_production".into(),
            Value::Array(
                cfg.production_audit()
                    .into_iter()
                    .map(Value::from)
                    .collect(),
            ),
        );
        // Inserted after redaction: `redact_credentials` treats unparseable
        // strings as potentially-secret, and "ok" is not a path to hide.
        obj.insert("validation".into(), json!("ok"));
        obj.insert(
            "backends".into(),
            json!({
                "sqlite": cfg!(feature = "sqlite"),
                "postgres": cfg!(feature = "postgres"),
                "redis": cfg!(feature = "redis"),
            }),
        );
        obj.insert("config_sha256".into(), cfg.hash().into());
    }
    v
}

// ------------------------------------------------------------------ misc

/// Render a bounded, aligned text table (CLI default output). Cells are
/// stringified already; no width heuristics beyond trimming to 64 chars.
pub fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    const MAX_ROWS: usize = 1000;
    const MAX_CELL: usize = 64;
    let clipped = |s: &str| -> String {
        if s.chars().count() <= MAX_CELL {
            s.to_string()
        } else {
            let cut: String = s.chars().take(MAX_CELL.saturating_sub(1)).collect();
            format!("{cut}…")
        }
    };
    let shown = rows.len().min(MAX_ROWS);
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for r in rows.iter().take(shown) {
        for (i, c) in r.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(clipped(c).chars().count());
            }
        }
    }
    let mut out = String::new();
    for (i, h) in headers.iter().enumerate() {
        out.push_str(h);
        if i + 1 < headers.len() {
            let pad = widths[i].saturating_sub(h.chars().count()) + 2;
            out.extend(std::iter::repeat_n(' ', pad));
        }
    }
    out.push('\n');
    for r in rows.iter().take(shown) {
        for (i, c) in r.iter().enumerate() {
            let c = clipped(c);
            out.push_str(&c);
            if i + 1 < r.len() && i < widths.len() {
                let pad = widths[i].saturating_sub(c.chars().count()) + 2;
                out.extend(std::iter::repeat_n(' ', pad));
            }
        }
        out.push('\n');
    }
    if rows.len() > shown {
        out.push_str(&format!("… {} more rows (bounded)\n", rows.len() - shown));
    }
    out
}

/// Compact single-line snapshot for logs (`harvest status --log`).
pub fn snapshot_log_line(snap: &Value) -> String {
    let g = |path: &[&str]| -> String {
        let mut cur = snap;
        for p in path {
            cur = match cur.get(p) {
                Some(v) => v,
                None => return "-".into(),
            };
        }
        match cur {
            Value::String(s) => s.clone(),
            v => v.to_string(),
        }
    };
    format!(
        "run={} status={} uptime_ms={} written={} quarantined={} events={}",
        g(&["engine", "run"]),
        g(&["engine", "status"]),
        g(&["engine", "uptime_ms"]),
        g(&["accounting", "records_written"]),
        g(&["accounting", "records_quarantined"]),
        g(&["events", "published"]),
    )
}

// ----------------------------------------------------------- admin client

/// Reply from a raw admin-HTTP call: status line + body, nothing else.
#[derive(Debug)]
pub struct AdminReply {
    pub status: u16,
    pub body: String,
}

impl AdminReply {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    pub fn json(&self) -> Result<Value, ApiError> {
        serde_json::from_str(&self.body)
            .map_err(|_| ApiError::unavailable("admin reply is not valid JSON"))
    }
}

/// Minimal one-shot HTTP/1.0 client for the loopback admin surface. std
/// only — no TLS, no pooling, bounded reads (4 MiB), fixed 3 s timeout.
/// This is a debugging client for an operator's box, not a library-grade
/// transport, and it is documented as such.
#[derive(Debug, Clone)]
pub struct AdminClient {
    pub addr: String,
    pub token: Option<String>,
}

impl AdminClient {
    pub fn new(addr: impl Into<String>, token: Option<String>) -> Self {
        Self {
            addr: addr.into(),
            token,
        }
    }

    pub fn get(&self, path_and_query: &str) -> Result<AdminReply, ApiError> {
        let host = self
            .addr
            .split(':')
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::invalid("empty admin address"))?;
        // Validate the path is what we think it is before touching the wire
        // (no CRLF smuggling, no oversized target line).
        if path_and_query.len() > 1024 || !path_and_query.bytes().all(|b| (0x20..0x7f).contains(&b))
        {
            return Err(ApiError::invalid("illegal admin request path"));
        }
        let socket = TcpStream::connect((host, self.port())).map_err(|_| {
            ApiError::unavailable(format!("admin API unreachable at {}", self.addr))
        })?;
        let _ = socket.set_read_timeout(Some(Duration::from_secs(3)));
        let _ = socket.set_write_timeout(Some(Duration::from_secs(3)));
        let req = match &self.token {
            Some(t) => format!(
                "GET {path_and_query} HTTP/1.0\r\nHost: {host}\r\nAuthorization: Bearer {t}\r\nAccept: application/json\r\n\r\n"
            ),
            None => format!(
                "GET {path_and_query} HTTP/1.0\r\nHost: {host}\r\nAccept: application/json\r\n\r\n"
            ),
        };
        Self::exchange(socket, req.as_bytes())
    }

    fn port(&self) -> u16 {
        self.addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9090)
    }

    fn exchange(mut socket: TcpStream, req: &[u8]) -> Result<AdminReply, ApiError> {
        socket
            .write_all(req)
            .map_err(|_| ApiError::unavailable("admin API write failed"))?;
        let mut raw = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        let t0 = Instant::now();
        loop {
            if raw.len() > 4 * 1024 * 1024 || t0.elapsed() > Duration::from_secs(5) {
                return Err(ApiError::unavailable(
                    "admin API response unbounded or slow",
                ));
            }
            match socket.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => raw.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => break,
            }
        }
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let mut lines = head.lines();
        let status = lines
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| ApiError::unavailable("admin API returned no status line"))?;
        Ok(AdminReply {
            status,
            body: body.to_string(),
        })
    }
}

/// `GET addr+path` and, for 401/403/503, surface the *server's* error
/// message (which is safe by construction) instead of a parse error.
pub fn admin_get_json(addr: &str, path: &str, token: Option<String>) -> Result<Value, ApiError> {
    let reply = AdminClient::new(addr, token).get(path)?;
    let v = reply.json()?;
    if !reply.ok() {
        let msg = v
            .get("error")
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("request failed")
            .to_string();
        return Err(ApiError {
            status: reply.status,
            code: "remote_error",
            message: msg,
        });
    }
    Ok(v)
}

/// Build a query string from present-only params, percent-encoding values.
pub fn build_query(params: &[(&str, Option<String>)]) -> String {
    let parts: Vec<String> = params
        .iter()
        .filter_map(|(k, v)| {
            v.as_ref().map(|v| {
                let enc: String = v
                    .bytes()
                    .map(|b| match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => {
                            (b as char).to_string()
                        }
                        _ => format!("%{b:02X}"),
                    })
                    .collect();
                format!("{k}={enc}")
            })
        })
        .collect();
    parts.join("&")
}

/// Static operator docs for the config surface: (path, type, default,
/// summary). Kept here (next to the API models it feeds) and pinned by a
/// test that every path resolves in a serialized `Config::default()` — so
/// the docs table cannot drift from the struct.
pub const CONFIG_KNOBS: &[(&str, &str, &str, &str)] = &[
    (
        "name",
        "string",
        "\"harvest\"",
        "run label recorded in history, events, and alert payloads",
    ),
    (
        "user_agent",
        "string",
        "harvest/0.1 (…)",
        "honest bot identity; validation rejects fake-browser UAs",
    ),
    (
        "respect_robots",
        "bool",
        "true",
        "hard refusal switch; false only for written-permission sources",
    ),
    (
        "concurrency",
        "usize",
        "4",
        "fetch worker count (supervisor keeps them bounded)",
    ),
    (
        "request_timeout_secs",
        "u64",
        "20",
        "per-request response timeout",
    ),
    (
        "connect_timeout_secs",
        "u64",
        "10",
        "TCP+TLS connect budget",
    ),
    (
        "config_version",
        "u32",
        "2",
        "schema stamp; v1 files migrate automatically, newer ones refuse to load",
    ),
    (
        "max_retries",
        "u32",
        "3",
        "transport/5xx retries per request",
    ),
    (
        "max_inflight_per_proxy",
        "u64",
        "0",
        "simultaneous requests per proxy egress; over-cap attempts are retryable",
    ),
    (
        "max_records_per_run",
        "u64",
        "0",
        "cap on records written per run; the scheduler drains gracefully at the cap",
    ),
    (
        "retry_base_ms",
        "u64",
        "500",
        "exponential retry curve base (60·2^n capped by max_single_wait)",
    ),
    (
        "max_single_wait_ms",
        "u64",
        "15000",
        "never sleep longer than this inside one retry loop",
    ),
    (
        "max_response_bytes",
        "u64",
        "16777216",
        "hard body cap; over it the URL fails permanently (no retries)",
    ),
    (
        "max_pages",
        "usize",
        "500",
        "global page budget per run (all targets)",
    ),
    (
        "default_policy.requests_per_sec",
        "f64",
        "1.0",
        "sustained rps to one host",
    ),
    (
        "default_policy.burst",
        "u32",
        "2",
        "token-bucket burst capacity",
    ),
    (
        "default_policy.min_interval_ms",
        "u64",
        "500",
        "absolute minimum gap between calls to one host",
    ),
    (
        "block_backoff_secs",
        "u64",
        "60",
        "backoff after a 401/403/429/451; doubles per consecutive block",
    ),
    (
        "block_breaker_count",
        "u32",
        "5",
        "blocks in one run before the domain is abandoned",
    ),
    ("robots_ttl_secs", "u64", "3600", "robots.txt cache TTL"),
    (
        "dlq_path",
        "path|null",
        "null",
        "dead-letter file; run --retry-dlq re-seeds it",
    ),
    (
        "dlq_max_attempts",
        "u32",
        "3",
        "requeue cap per URL before it stays dead",
    ),
    (
        "redis_url",
        "url|null",
        "null",
        "shared queue for harvest push/worker (redis feature)",
    ),
    (
        "ckpt_path",
        "path|null",
        "null",
        "checkpoint file for pause/resume of long crawls",
    ),
    (
        "ckpt_every_pages",
        "usize",
        "250",
        "checkpoint cadence by page count",
    ),
    (
        "ckpt_every_secs",
        "u64",
        "30",
        "checkpoint cadence by wall time",
    ),
    (
        "revalidate_store",
        "path|null",
        "null",
        "ETag/Last-Modified store: repeat crawls pay 304s",
    ),
    (
        "revalidate_ttl_secs",
        "u64",
        "0",
        "re-check validators younger than this; 0 = never expire",
    ),
    (
        "audit_path",
        "path|null",
        "null",
        "per-host politeness receipt written at run end",
    ),
    (
        "attest_key_path",
        "path|null",
        "null",
        "HMAC key file; set => every artifact gets a detached .sig",
    ),
    (
        "admin_token",
        "string|null",
        "null",
        "bearer token for every admin endpoint; /health+/live exempt",
    ),
    (
        "metrics_addr",
        "host:port",
        "127.0.0.1:9678",
        "admin API bind; empty disables the server",
    ),
    (
        "events_path",
        "path|null",
        "null",
        "structured-event JSONL log (bounded by events_max_bytes)",
    ),
    (
        "retry_deterministic",
        "bool",
        "false",
        "seed retries from URL so runs are replayable",
    ),
    (
        "scheduling.mode",
        "string",
        "\"fifo\"",
        "fifo | priority | fair target interleaving",
    ),
    (
        "scheduling.max_queue_depth",
        "u32",
        "0",
        "bounded queue; 0 = unbounded (legacy)",
    ),
    (
        "scheduling.per_target_inflight",
        "u32",
        "0",
        "per-target in-flight cap; 0 = off",
    ),
    (
        "scheduling.worker_restarts",
        "u32",
        "0",
        "extra spawn attempts after a worker task dies",
    ),
    (
        "scheduling.job_max_age_secs",
        "u64",
        "0",
        "expire queued jobs older than this; 0 = never",
    ),
    (
        "scheduling.retry_budget_per_min",
        "u32",
        "0",
        "global retry token bucket; 0 = unlimited",
    ),
    (
        "transport_breaker",
        "object|null",
        "null",
        "per-host transport circuit (unset = disabled)",
    ),
    (
        "transport_breaker.min_consecutive",
        "u32",
        "3",
        "transport/5xx failures before the circuit opens",
    ),
    (
        "transport_breaker.cooldown_secs",
        "u64",
        "60",
        "skip window after opening; then one probe",
    ),
    (
        "transport_breaker.recovery_successes",
        "u32",
        "2",
        "probe successes needed to close again",
    ),
    (
        "alerts.webhook_url",
        "url|null",
        "null",
        "POST target for SLO alerts (empty disables)",
    ),
    (
        "alerts.events",
        "[string]",
        "[]",
        "run_error | no_data | high_block_rate | quarantine_surge",
    ),
    (
        "alerts.min_records",
        "u64",
        "0",
        "no_data floor: written+revalidated below this fires",
    ),
    (
        "alerts.max_block_rate",
        "f64",
        "0",
        "blocked/requests ratio that pages; 0 disables",
    ),
    (
        "alerts.quarantine_threshold",
        "u64",
        "0",
        "quarantine_surge floor; 0 = event disabled",
    ),
    (
        "alerts.cooldown_secs",
        "u64",
        "0",
        "min seconds between webhook deliveries per event; 0 = every fire",
    ),
    (
        "alerts.min_severity",
        "string",
        "\"info\"",
        "delivery floor: info | warning | error",
    ),
    ("schedule.every_secs", "u64", "3600", "serve cadence"),
    (
        "schedule.runs",
        "usize",
        "0",
        "run count cap for serve; 0 = until stopped",
    ),
    (
        "schedule.history_path",
        "path",
        "out/runs.jsonl",
        "append-only run history (/runs, /recent read it)",
    ),
];

/// Resolve a dotted path inside a (possibly redacted) serialized config.
pub fn knob_lookup(v: &Value, path: &str) -> Option<Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = cur.get(seg)?;
    }
    Some(cur.clone())
}

/// Rows for `harvest config explain`: docs + (when a loaded config is
/// given) the current redacted value and whether it came from the file,
/// the environment, or stayed at its default.
pub fn explain_config(cfg: &Value, sources: Option<&Value>, filter: Option<&str>) -> Vec<Value> {
    CONFIG_KNOBS
        .iter()
        .filter(|(path, ..)| filter.map(|f| path.contains(f)).unwrap_or(true))
        .map(|(path, ty, default, summary)| {
            let mut row = json!({
                "path": path,
                "type": ty,
                "default": default,
                "summary": summary,
            });
            if let Some(cur) = knob_lookup(cfg, path) {
                row["current"] = cur;
            }
            if let Some(src) = sources {
                let top = path.split('.').next().unwrap_or(path);
                if let Some(s) = src.get(top) {
                    row["source"] = s.clone();
                }
            }
            row
        })
        .collect()
}

// ------------------------------------------------------------------ tests

#[cfg(test)]
mod tests;
