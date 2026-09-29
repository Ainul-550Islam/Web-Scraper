//! Typed identifiers and value semantics shared by every layer.
//!
//! Before this module, the same concepts (target names, proxy ids, redis key
//! prefixes, content fingerprints, partition labels) were validated ad hoc
//! in config, engine, dist, sink, and the CLI — with slightly different
//! rules per site. These types are the single authority: parse once at the
//! boundary, then carry the invariant everywhere.
//!
//! Design rules (deliberate, so this never rots into ceremony):
//!
//! * **Only wrap where a rule exists.** A `SlugId` is a non-empty, ASCII,
//!   filesystem-and-redis-safe token; the parse/validate rule is the value
//!   add. Plain numbers already typed at their definitions (e.g.
//!   [`crate::job::JobId`]) are not re-wrapped here.
//! * **Stringly-compatible serde.** Serialized as the bare string, so JSON
//!   formats (history, quarantine, DLQ, admin API) are unchanged — these
//!   types strengthen code, not wire formats.
//! * **Deterministic ordering** via the inner string, so any listing keyed
//!   by an id sorts stably across runs and machines.
//! * **Timestamps** centralize the epoch-seconds convention the whole repo
//!   already uses (`i64` unix seconds), plus the UTC calendar math the
//!   dataset sink needs — no dependency, no `SystemTime` boilerplate.

use std::fmt;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Why a raw string failed to become a typed value. Message style matches the
/// config layer: always names the offending input and the accepted shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdError(pub String);

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for IdError {}

/// The shared character class: `a-z 0-9 . _ -`, 1..=64 bytes, must start with
/// alphanumeric, and may never be `..` or contain path/URL/redis-key
/// separators. Validated on every character so CRLF or `/` can never ride
/// through into a filename, a Redis key, or a log line.
fn slug_ok(s: &str, what: &str) -> Result<(), IdError> {
    if s.is_empty() {
        return Err(IdError(format!("{what} must not be empty")));
    }
    if s.len() > 64 {
        return Err(IdError(format!(
            "{what} `{}` is {} bytes (limit: 64)",
            &s[..s.len().min(24)],
            s.len()
        )));
    }
    // `.` is a legal middle byte, but dot-lead/`..`/dot-tail are reserved
    // for path semantics — an id that turns into a path component must not
    // play with them.
    if s.contains('/')
        || s.contains('\\')
        || s.contains(':')
        || s.contains("..")
        || s.ends_with('.')
    {
        return Err(IdError(format!(
            "{what} `{s}` must not contain path or key separators (or stray dots)"
        )));
    }
    let mut chars = s.chars();
    let first = chars.next().expect("checked non-empty");
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(IdError(format!(
            "{what} `{s}` must start with a lowercase letter or digit"
        )));
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
            return Err(IdError(format!(
                "{what} `{s}` has an illegal byte `{c}` (allowed: a-z 0-9 . _ -)"
            )));
        }
    }
    Ok(())
}

/// Defines a newtype slug id with the shared rules. Keeps the module free of
/// five copies of identical impl blocks; every macro invocation is a real,
/// separately-validated concept, not a wrapper of a wrapper.
macro_rules! slug_id {
    ($name:ident, $what:literal, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap. Rules: 1..=64 bytes, lowercase ASCII slugs
            /// (`a-z 0-9 . _ -`), must start alphanumeric; no path/key
            /// separators, never `..`. See the module docs.
            pub fn parse(s: &str) -> Result<Self, IdError> {
                slug_ok(s, $what)?;
                Ok(Self(s.to_string()))
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
            /// Unchecked constructor for values that came from a config
            /// validated elsewhere in the same process (config keeps ids as
            /// plain strings for wire compatibility; `Config::validate`
            /// checks them — this carries that check forward without
            /// re-parsing hot paths).
            /// Wrap a string already known to pass `parse` (hot paths, tests).
            pub fn from_validated(s: String) -> Self {
                Self(s)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.serialize_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                Self::parse(&s).map_err(serde::de::Error::custom)
            }
        }
    };
}

slug_id!(
    TargetId,
    "target id",
    "A crawl target's stable name (config `targets[].id`)."
);
slug_id!(
    ProxyId,
    "proxy id",
    "A proxy-pool entry id (config `proxies[].id`, or the synthetic `direct`)."
);
slug_id!(
    WorkerId,
    "worker id",
    "A distributed worker's name; keys lease/heartbeat records and logs."
);
/// Validity for a *Redis* key token (queue prefixes). Redis namespaces
/// conventionally use `:` — keys are not paths — so this is deliberately
/// wider than the slug rules while still excluding every byte that could
/// break out of a key pattern (`*`, `}`, whitespace, `;`, quotes, …).
pub fn is_safe_key_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
}
slug_id!(
    RunName,
    "run name",
    "The human label of a run (`name` in the config); keys history lines."
);

/// The content fingerprint of a document body (sha256 hex, lowercase, 64
/// chars) — dedupe key, quarantine linkage, export `unique_urls` set.
/// Parsing is enforced at the boundary so a truncated/corrupted fp cannot
/// silently split a dedupe set.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RecordId(String);

impl RecordId {
    /// Wrap a lowercase hex sha256.
    pub fn parse(s: &str) -> Result<Self, IdError> {
        if s.len() != 64
            || !s
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(IdError(
                "record id must be 64 lowercase hex chars (a sha256 fingerprint)".into(),
            ));
        }
        Ok(Self(s.to_string()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl FromStr for RecordId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}
impl Serialize for RecordId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for RecordId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// One claim of one job by one worker. Format is generated, not user input:
/// `<worker-id>-s<counter>-<8 hex>`; parsing validates the shape so a lease
/// id that drifted through a queue can never name a foreign key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LeaseId(String);

impl LeaseId {
    pub fn new(worker: &WorkerId, seq: u64, salt: u32) -> Self {
        Self(format!("{worker}-s{seq}-{salt:08x}"))
    }
    pub fn parse(s: &str) -> Result<Self, IdError> {
        let tail = s.rsplit_once('-').map(|(_, t)| t).unwrap_or("");
        let mid_ok = s
            .rsplit_once('-')
            .and_then(|(head, _)| head.rsplit_once("-s"))
            .is_some_and(|(w, n)| {
                !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && {
                    WorkerId::parse(w).is_ok()
                }
            });
        let hex_ok = tail.len() == 8
            && tail
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        let len_ok = (8..=96).contains(&s.len());
        if mid_ok && hex_ok && len_ok {
            Ok(Self(s.to_string()))
        } else {
            Err(IdError(
                "lease id must look like <worker-id>-s<seq>-<8 hex>".into(),
            ))
        }
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for LeaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Unix-seconds timestamp — the repo-wide convention for every `ts` field
/// (history lines, records, events, alert state). Single place for
/// "what is it now", so clock reads can't drift formats between layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Timestamp(pub i64);

impl Timestamp {
    pub fn now() -> Self {
        Self(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        )
    }
    pub fn as_secs(self) -> i64 {
        self.0
    }
    /// `YYYY-MM-DD` in UTC — shared by dataset partitioning and reports.
    pub fn utc_day(self) -> String {
        let (y, m, d) = civil_from_days(self.0.div_euclid(86_400));
        format!("{y:04}-{m:02}-{d:02}")
    }
    /// Compact `YYYYMMDD` for file-safe labels (export names, CLI filters).
    pub fn utc_day_compact(self) -> String {
        let (y, m, d) = civil_from_days(self.0.div_euclid(86_400));
        format!("{y:04}{m:02}{d:02}")
    }
}
impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Milliseconds — the repo-wide duration unit for metrics/history. A newtype
/// here exists for exactly one reason: formatting and fields can no longer
/// accidentally receive a value in the wrong scale without a name.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
pub struct DurationMs(pub u64);

impl DurationMs {
    pub fn as_millis(self) -> u64 {
        self.0
    }
    pub fn from_std(d: std::time::Duration) -> Self {
        Self(d.as_millis() as u64)
    }
}
impl fmt::Display for DurationMs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}ms", self.0)
    }
}

/// Which UTC day (or flat `parts/`) a dataset record belongs to — the
/// single definition of the dataset partition layout, shared by
/// `sink::DatasetSink`, the `harvest datasets` CLI filter, and export
/// naming. `dt=YYYY-MM-DD` directory labels, as on the wire since day one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetPartition {
    Day { y: i64, m: u32, d: u32 },
    Flat,
}

impl DatasetPartition {
    pub fn for_record_ts(ts_secs: i64) -> Self {
        let (y, m, d) = civil_from_days(ts_secs.div_euclid(86_400));
        Self::Day { y, m, d }
    }
    pub fn dir_name(&self) -> String {
        match self {
            Self::Day { y, m, d } => format!("dt={y:04}-{m:02}-{d:02}"),
            Self::Flat => "parts".into(),
        }
    }
    /// Parse a CLI `--dt` selector: full label, `YYYY-MM-DD`, or `YYYYMMDD`.
    pub fn parse_selector(s: &str) -> Result<Self, IdError> {
        let core = s.strip_prefix("dt=").unwrap_or(s);
        let digits: String = core.chars().filter(|c| *c != '-').collect();
        if digits.len() == 8 && digits.bytes().all(|b| b.is_ascii_digit()) {
            let y: i64 = digits[0..4].parse().expect("digits checked");
            let m: u32 = digits[4..6].parse().expect("digits checked");
            let d: u32 = digits[6..8].parse().expect("digits checked");
            if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
                return Err(IdError(format!("`{s}` has an impossible month/day")));
            }
            return Ok(Self::Day { y, m, d });
        }
        Err(IdError(format!(
            "`{s}` is not a partition selector (expected dt=YYYY-MM-DD)"
        )))
    }
}

/// Days-since-epoch → proleptic Gregorian `y/m/d` (Howard Hinnant's civil
/// algorithm). Moved here from the sink so exports, partitions, and reports
/// compute the UTC calendar identically.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d)
}

/// Request correlation id — moved here so every surface that stamps
/// trace ids (admin API, alerts, audit, ops ring) validates against the same
/// rule.
/// Request correlation id. An incoming `X-Request-Id` is accepted only if it
/// is 1..=64 chars of `[A-Za-z0-9._:-]` (header-injection and log-spam
/// resistant); anything else is discarded and a fresh id is generated. The
/// id appears in the response header, the error JSON, and the access log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrelationId(String);

impl CorrelationId {
    pub fn new() -> Self {
        Self(Self::generate())
    }

    /// Accept-or-generate: never returns the client's garbage back to it.
    pub fn from_incoming(v: Option<&str>) -> Self {
        match v {
            Some(s) if Self::valid(s) => Self(s.to_string()),
            _ => Self(Self::generate()),
        }
    }

    pub fn valid(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 64
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
    }

    fn generate() -> String {
        use std::sync::atomic::Ordering;
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as u64)
            .unwrap_or(0);
        // xor keeps it unpredictable-ish without pulling a rng dependency
        // (ids are log join keys, not capability tokens — auth is the
        // bearer token, never this).
        format!(
            "req-{:016x}",
            (nanos << 20) ^ n ^ (std::process::id() as u64)
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for CorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests;
