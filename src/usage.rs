//! Usage metering for the managed customer API.
//!
//! Three units, one contract:
//!
//! * [`ledger`] — the append-only, hash-chained JSONL record of every
//!   metered request. This is the billing source of truth.
//! * [`aggregate`] — pure folds over the ledger: per-key rows, per-day
//!   series, and the monthly unit count the quota check rebuilds from.
//! * [`export`] — bounded CSV/JSONL exports with a verifying manifest.
//!
//! The ledger is written through `fsutil` (private permissions, atomic
//! append, no symlink adoption) and rotated by UTC month, so it stays
//! bounded on disk while remaining complete and verifiable.
//!
//! See `docs/modules/usage.md`.

pub mod aggregate;
pub mod export;
pub mod ledger;

#[cfg(test)]
mod tests;

pub use aggregate::{
    day_of, monthly_units, monthly_units_by_key, summarize, DayRow, UsageFilter, UsageReport,
    UsageRow,
};
pub use export::{
    build_report, export_csv, export_jsonl, export_report_csv, report_json, ExportResult,
    EXPORT_MAX_ROWS,
};
pub use ledger::{
    month_file_name, month_of, LedgerEntry, RenderMode, UsageLedger, LEDGER_SCHEMA_VERSION,
};
