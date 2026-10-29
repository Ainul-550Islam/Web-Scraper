//! Aggregations over the usage ledger.
//!
//! The ledger is the raw record; this module turns it into the shapes the
//! API and the console actually render: per-key rows, per-day series and the
//! single number the quota check needs. Everything here is a pure fold over
//! an ordered slice, so it is deterministic and cheap to test.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::usage::ledger::{month_of, LedgerEntry};

/// Which entries to include. Every field is optional; an empty filter is the
/// whole ledger (bounded by `limit`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageFilter {
    pub tenant_id: Option<String>,
    pub key_id: Option<String>,
    pub since_ms: Option<u64>,
    pub until_ms: Option<u64>,
    /// Row cap for `by_key`. Largest value accepted by the API.
    pub limit: usize,
}

impl UsageFilter {
    pub fn matches(&self, e: &LedgerEntry) -> bool {
        if let Some(t) = &self.tenant_id {
            if &e.tenant_id != t {
                return false;
            }
        }
        if let Some(k) = &self.key_id {
            if &e.key_id != k {
                return false;
            }
        }
        if let Some(s) = self.since_ms {
            if e.ts_ms < s {
                return false;
            }
        }
        if let Some(u) = self.until_ms {
            if e.ts_ms > u {
                return false;
            }
        }
        true
    }

    /// Apply the filter and take at most `limit` rows.
    pub fn apply<'a>(
        &self,
        entries: impl Iterator<Item = &'a LedgerEntry>,
    ) -> Vec<&'a LedgerEntry> {
        let mut out: Vec<&LedgerEntry> = entries.filter(|e| self.matches(e)).collect();
        // Zero is "no cap": the API always sets an explicit bound, and a
        // filter built from `Default` should not silently drop everything.
        if self.limit > 0 && out.len() > self.limit {
            out.truncate(self.limit);
        }
        out
    }
}

/// One metered key's activity over the filtered window.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct UsageRow {
    pub key_id: String,
    pub tenant_id: String,
    pub requests: u64,
    pub ok: u64,
    pub refused: u64,
    pub errors: u64,
    pub units: u64,
    pub bytes: u64,
    pub render_none: u64,
    pub render_browser: u64,
    pub screenshots: u64,
    pub first_ts_ms: u64,
    pub last_ts_ms: u64,
}

impl UsageRow {
    fn note(&mut self, e: &LedgerEntry) {
        self.requests += 1;
        self.units += u64::from(e.units);
        self.bytes += e.bytes;
        self.first_ts_ms = self.first_ts_ms.min(e.ts_ms);
        self.last_ts_ms = self.last_ts_ms.max(e.ts_ms);
        match e.status.as_str() {
            "ok" => self.ok += 1,
            "refused" => self.refused += 1,
            _ => self.errors += 1,
        }
        match e.render {
            crate::usage::ledger::RenderMode::None => self.render_none += 1,
            crate::usage::ledger::RenderMode::Browser => self.render_browser += 1,
        }
        if e.screenshot {
            self.screenshots += 1;
        }
    }

    /// Fold several rows into one. `key_id`/`tenant_id` come from the first
    /// row; the counters are summed.
    pub fn merge(&mut self, other: &UsageRow) {
        self.requests += other.requests;
        self.ok += other.ok;
        self.refused += other.refused;
        self.errors += other.errors;
        self.units += other.units;
        self.bytes += other.bytes;
        self.render_none += other.render_none;
        self.render_browser += other.render_browser;
        self.screenshots += other.screenshots;
        self.first_ts_ms = self.first_ts_ms.min(other.first_ts_ms);
        self.last_ts_ms = self.last_ts_ms.max(other.last_ts_ms);
    }
}

/// One day's totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DayRow {
    /// UTC day, `YYYY-MM-DD`.
    pub day: String,
    pub requests: u64,
    pub units: u64,
    pub bytes: u64,
}

/// The full report served by `GET /v1/usage`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageReport {
    pub since_ms: u64,
    pub until_ms: u64,
    pub generated_at_ms: u64,
    /// Totals across every key in the window.
    pub totals: UsageRow,
    /// Per-key rows, largest units first.
    pub by_key: Vec<UsageRow>,
    /// Per-UTC-day series, oldest first.
    pub by_day: Vec<DayRow>,
    /// Distinct keys in the window.
    pub key_count: usize,
}

/// Fold the ledger into a report. `entries` must be in ascending `seq`
/// order (which is what `UsageLedger::read_all` returns).
pub fn summarize(entries: &[LedgerEntry], filter: &UsageFilter) -> UsageReport {
    let mut by_key: BTreeMap<(&str, &str), UsageRow> = BTreeMap::new();
    let mut by_day: BTreeMap<String, DayRow> = BTreeMap::new();
    let mut totals = UsageRow {
        key_id: "*".to_string(),
        tenant_id: "*".to_string(),
        ..UsageRow::default()
    };
    let mut min_ts = u64::MAX;
    let mut max_ts = 0u64;
    let mut counted = 0u64;

    for e in entries {
        if !filter.matches(e) {
            continue;
        }
        let row = by_key
            .entry((e.key_id.as_str(), e.tenant_id.as_str()))
            .or_insert_with(|| UsageRow {
                key_id: e.key_id.clone(),
                tenant_id: e.tenant_id.clone(),
                ..UsageRow::default()
            });
        row.note(e);
        totals.note(e);

        let day = day_of(e.ts_ms);
        let dr = by_day.entry(day.clone()).or_insert_with(|| DayRow {
            day,
            ..DayRow::default()
        });
        dr.requests += 1;
        dr.units += u64::from(e.units);
        dr.bytes += e.bytes;

        min_ts = min_ts.min(e.ts_ms);
        max_ts = max_ts.max(e.ts_ms);
        counted += 1;
    }

    let mut rows: Vec<UsageRow> = by_key.into_values().collect();
    rows.sort_by(|a, b| b.units.cmp(&a.units).then_with(|| a.key_id.cmp(&b.key_id)));
    if filter.limit > 0 && rows.len() > filter.limit {
        rows.truncate(filter.limit);
    }
    let mut days: Vec<DayRow> = by_day.into_values().collect();
    days.sort_by(|a, b| a.day.cmp(&b.day));

    let key_count = rows.len();
    UsageReport {
        since_ms: if counted == 0 { 0 } else { min_ts },
        until_ms: max_ts,
        generated_at_ms: now_ms(),
        totals,
        by_key: rows,
        by_day: days,
        key_count,
    }
}

/// UTC day string `YYYY-MM-DD` for an epoch millisecond timestamp.
pub fn day_of(ts_ms: u64) -> String {
    let (y, m, d) = crate::ids::civil_from_days((ts_ms / 1000).div_euclid(86_400) as i64);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Total units a key spent in a UTC month. Used to rebuild the in-memory
/// quota counter after a restart, so a restart cannot hand a tenant a free
/// month.
pub fn monthly_units(entries: &[LedgerEntry], key_id: &str, month: i64) -> u64 {
    entries
        .iter()
        .filter(|e| e.key_id == key_id && month_of(e.ts_ms) == month)
        .map(|e| u64::from(e.units))
        .sum()
}

/// Same, for every key, as a map.
pub fn monthly_units_by_key(entries: &[LedgerEntry], month: i64) -> BTreeMap<String, u64> {
    let mut out: BTreeMap<String, u64> = BTreeMap::new();
    for e in entries {
        if month_of(e.ts_ms) == month {
            *out.entry(e.key_id.clone()).or_insert(0) += u64::from(e.units);
        }
    }
    out
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
