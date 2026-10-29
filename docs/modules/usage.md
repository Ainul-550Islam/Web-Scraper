# Module `usage`

Source: [`src/usage/`](../../src/usage/) · unit tests: [`src/usage/tests.rs`](../../src/usage/tests.rs)

The metering ledger and everything read out of it. One file per calendar
month, append-only, hash-chained; the report is a fold over the entries, so the
numbers can always be recomputed from the record.

Public surface:

- `src/usage/ledger.rs`
  - `struct LedgerEntry` — `seq`, `ts_ms`, `tenant_id`, `key_id`, `action`,
    `units`, `bytes`, `status`, `render`, `screenshot`, `request_id`, `prev_hash`,
    `hash`; `compute_hash`, `continues_chain`
  - `struct UsageLedger` — `open`, `dir`, `seq`, `last_hash`, `read_all`,
    `verify`, `append`, `record`
  - `enum RenderMode` — `none`, `browser`; `as_str`, `parse`
  - `fn month_file_name`, `fn month_of`, `fn override_now`, `fn clear_now_override`
  - `const LEDGER_SCHEMA_VERSION`, `const LEDGER_ROTATE_BYTES`
- `src/usage/aggregate.rs`
  - `struct UsageFilter` — `tenant_id`, `key_id`, `since_ms`, `until_ms`, `limit`;
    `matches`, `apply`
  - `struct UsageRow` — per-key counters; `merge`
  - `struct DayRow` — `day`, `requests`, `units`, `bytes`
  - `struct UsageReport` — `since_ms`, `until_ms`, `generated_at_ms`, `totals`,
    `by_key`, `by_day`, `key_count`
  - `fn summarize`, `fn day_of`, `fn monthly_units`, `fn monthly_units_by_key`
- `src/usage/export.rs`
  - `fn export_csv`, `fn export_jsonl`, `fn export_report_csv`,
    `fn report_json`, `fn build_report`
  - `struct ExportResult` — `schema`, `path`, `manifest`, `rows`, `bytes`,
    `sha256`, `truncated`
  - `const EXPORT_MAX_ROWS`

### The chain

`hash` is `sha256(seq || prev_hash || canonical entry)`, so an entry cannot be
edited without breaking every entry after it. `verify()` walks the ledger and
returns the number of entries that hold; it reports where the chain first
breaks.

An append-only chain detects an edited, dropped-out-of-the-middle, or
reordered entry. It does **not** detect a truncated tail: removing the last
few entries leaves the remaining ones perfectly consistent. That is why
`UsageLedger::seq()` and the per-month file rotation exist — a caller that
needs the guarantee also pins the head hash it observed (`chain` in every
response, `ledger_head` in the usage report).

`override_now` exists for tests: `UsageLedger::open` derives the current month
from the wall clock, so a test that writes synthetic timestamps has to say
which month it means first.

### Export

Exports are written with `fsutil::write_private_atomic` and get a manifest
(`.manifest.json`) recording the row count, the byte count and the SHA-256 of
the payload, so a delivered file can be checked against what was asked for.
Rows are capped at `EXPORT_MAX_ROWS` and the manifest says `truncated` when the
cap bit.
