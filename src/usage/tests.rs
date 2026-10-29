//! Tests for the usage ledger, its aggregations and its exports.

use sha2::Digest as _;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::usage::aggregate::{
    day_of, monthly_units, monthly_units_by_key, summarize, UsageFilter,
};
use crate::usage::export::{export_csv, export_jsonl, export_report_csv, EXPORT_MAX_ROWS};
use crate::usage::ledger::{
    clear_now_override, month_file_name, month_of, override_now, LedgerEntry, RenderMode,
    UsageLedger, LEDGER_SCHEMA_VERSION,
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("harvest-usage-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn entry(seq: u64, key: &str, units: u32, ts_ms: u64) -> LedgerEntry {
    LedgerEntry {
        seq,
        schema: LEDGER_SCHEMA_VERSION,
        prev_hash: String::new(),
        hash: String::new(),
        ts_ms,
        tenant_id: "t-1".to_string(),
        key_id: key.to_string(),
        request_id: format!("r-{seq}"),
        units,
        render: RenderMode::None,
        screenshot: false,
        proxy_region: None,
        status: "ok".to_string(),
        status_code: 200,
        bytes: 1_000,
        duration_ms: 5,
    }
}

fn record(ledger: &UsageLedger, key: &str, units: u32, ts_ms: u64) -> LedgerEntry {
    ledger
        .record(
            "t-1",
            key,
            &format!("r-{ts_ms}"),
            units,
            RenderMode::None,
            false,
            None,
            "ok",
            200,
            1_000,
            5,
        )
        .unwrap()
}

const DAY: u64 = 86_400_000;

// --------------------------------------------------------------- ledger

#[test]
fn appended_entries_read_back_unchanged() {
    let dir = TempDir::new("roundtrip");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    let a = record(&ledger, "k-1", 1, 1_000);
    let b = record(&ledger, "k-2", 2, 2_000);
    let read = ledger.read_all().unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], a);
    assert_eq!(read[1], b);
    assert_eq!(ledger.seq(), 2);
    assert_eq!(ledger.verify().unwrap(), 2);
}

#[test]
fn every_entry_continues_the_previous_hash() {
    let dir = TempDir::new("chain");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    let first = record(&ledger, "k-1", 1, 1_000);
    assert_eq!(first.prev_hash, "", "the genesis entry has no predecessor");
    assert_ne!(first.hash, "", "the genesis entry still has a digest");
    let second = record(&ledger, "k-1", 1, 2_000);
    assert_eq!(second.prev_hash, first.hash);
    assert_ne!(second.hash, first.hash);
    assert!(second.continues_chain(&first.hash));
    assert!(!second.continues_chain(&first.hash[..first.hash.len() - 1]));
}

#[test]
fn a_edited_entry_breaks_the_chain() {
    override_now(1_000);
    let dir = TempDir::new("tamper");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    record(&ledger, "k-1", 1, 1_000);
    record(&ledger, "k-1", 2, 2_000);
    let path = dir.path().join(month_file_name(month_of(1_000)));
    let text = std::fs::read_to_string(&path).unwrap();
    // Rewrite the second entry's units from 2 to 999 without touching the
    // stored digest.
    let tampered = text.replace("\"units\":2", "\"units\":999");
    assert_ne!(tampered, text, "fixture must actually change the file");
    std::fs::write(&path, tampered).unwrap();
    clear_now_override();

    let reopened = UsageLedger::open(dir.path());
    assert!(
        reopened.is_err(),
        "a tampered ledger must not open silently"
    );
}

#[test]
fn a_dropped_middle_entry_breaks_the_chain() {
    override_now(1_000);
    let dir = TempDir::new("drop");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    record(&ledger, "k-1", 1, 1_000);
    record(&ledger, "k-1", 2, 2_000);
    record(&ledger, "k-1", 3, 3_000);
    let path = dir.path().join(month_file_name(month_of(1_000)));
    let text = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    // Deleting a middle row leaves a valid head and a valid tail; only the
    // link between them is gone, which is exactly what the chain catches.
    // (Dropping the *tail* is not detectable from the file alone — an
    // append-only chain needs an external anchor for that — so it is not
    // claimed here.)
    std::fs::write(&path, format!("{}\n{}\n", lines[0], lines[2])).unwrap();
    clear_now_override();
    assert!(UsageLedger::open(dir.path()).is_err());
}

#[test]
fn an_empty_ledger_is_valid() {
    let dir = TempDir::new("empty");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    assert_eq!(ledger.read_all().unwrap().len(), 0);
    assert_eq!(ledger.seq(), 0);
    assert_eq!(ledger.last_hash(), "");
}

#[test]
fn a_reordered_file_breaks_the_chain() {
    override_now(1_000);
    let dir = TempDir::new("reorder");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    record(&ledger, "k-1", 1, 1_000);
    record(&ledger, "k-1", 2, 2_000);
    let path = dir.path().join(month_file_name(month_of(1_000)));
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    lines.reverse();
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    clear_now_override();
    assert!(UsageLedger::open(dir.path()).is_err());
}

#[test]
fn a_restart_continues_the_chain() {
    let dir = TempDir::new("restart");
    let first = UsageLedger::open(dir.path()).unwrap();
    let a = record(&first, "k-1", 1, 1_000);
    drop(first);

    let second = UsageLedger::open(dir.path()).unwrap();
    assert_eq!(second.seq(), 1, "the sequence continues across restarts");
    let b = record(&second, "k-1", 3, 2_000);
    assert_eq!(
        b.prev_hash, a.hash,
        "the new entry chains onto the old head"
    );
    assert_eq!(second.read_all().unwrap().len(), 2);
}

#[test]
fn month_rollover_starts_a_new_file() {
    let dir = TempDir::new("rollover");
    override_now(1_700_000_000_000);
    let ledger = UsageLedger::open(dir.path()).unwrap();
    record(&ledger, "k-1", 1, 1_700_000_000_000);
    let month_a = month_of(1_700_000_000_000);

    override_now(1_700_000_000_000 + 40 * DAY);
    record(&ledger, "k-1", 2, 1_700_000_000_000 + 40 * DAY);
    clear_now_override();

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.starts_with("usage-"))
        .collect();
    assert_eq!(names.len(), 2, "rollover must create a second file");
    assert!(names.contains(&month_file_name(month_a)));
    assert_ne!(
        month_of(1_700_000_000_000),
        month_of(1_700_000_000_000 + 40 * DAY)
    );

    let read = ledger.read_all().unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(
        read[1].prev_hash, read[0].hash,
        "the chain crosses the file boundary"
    );
}

#[test]
fn month_file_names_are_zero_padded_and_sortable() {
    assert_eq!(month_file_name(2026 * 12 + 9), "usage-2026-10.jsonl");
    assert_eq!(month_file_name(2025 * 12 + 11), "usage-2025-12.jsonl");
    assert_eq!(month_file_name(2026 * 12), "usage-2026-01.jsonl");
    assert_eq!(month_of(0), 1970 * 12);
}

#[test]
fn the_ledger_refuses_an_unwritable_root() {
    // A file where the ledger directory should be cannot be prepared.
    let dir = TempDir::new("badroot");
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"not a directory").unwrap();
    assert!(UsageLedger::open(&blocker).is_err());
}

#[test]
fn ledger_entries_reject_unknown_fields() {
    let raw = r#"{"seq":1,"schema":1,"prev_hash":"","hash":"x","ts_ms":1,
        "tenant_id":"t","key_id":"k","request_id":"r","units":1,"render":"none",
        "screenshot":false,"proxy_region":null,"status":"ok","status_code":200,
        "bytes":1,"duration_ms":1,"invented":true}"#;
    assert!(serde_json::from_str::<LedgerEntry>(raw).is_err());
}

// --------------------------------------------------------------- aggregate

fn ledger_with(entries: &[(u64, &str, u32, u64)]) -> (TempDir, Vec<LedgerEntry>) {
    let dir = TempDir::new("agg");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    let mut out = Vec::new();
    for (seq, key, units, ts) in entries {
        let mut e = entry(*seq, key, *units, *ts);
        e.seq = *seq;
        ledger.append(e.clone()).unwrap();
        out.push(ledger.read_all().unwrap().last().unwrap().clone());
    }
    (dir, out)
}

#[test]
fn totals_and_per_key_rows_are_correct() {
    let (_dir, entries) = ledger_with(&[
        (1, "k-1", 1, 1_000),
        (2, "k-1", 3, 2_000),
        (3, "k-2", 5, 3_000),
    ]);
    let report = summarize(&entries, &UsageFilter::default());
    assert_eq!(report.totals.requests, 3);
    assert_eq!(report.totals.units, 9);
    assert_eq!(report.totals.ok, 3);
    assert_eq!(report.totals.bytes, 3_000);
    assert_eq!(report.key_count, 2);
    // Largest units first.
    assert_eq!(report.by_key[0].key_id, "k-2");
    assert_eq!(report.by_key[0].units, 5);
    assert_eq!(report.by_key[1].units, 4);
}

#[test]
fn status_classes_are_separated() {
    let dir = TempDir::new("status");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    ledger
        .record(
            "t",
            "k",
            "r1",
            1,
            RenderMode::None,
            false,
            None,
            "ok",
            200,
            10,
            1,
        )
        .unwrap();
    ledger
        .record(
            "t",
            "k",
            "r2",
            1,
            RenderMode::None,
            false,
            None,
            "refused",
            403,
            10,
            1,
        )
        .unwrap();
    ledger
        .record(
            "t",
            "k",
            "r3",
            1,
            RenderMode::None,
            false,
            None,
            "error",
            500,
            10,
            1,
        )
        .unwrap();
    let entries = ledger.read_all().unwrap();
    let report = summarize(&entries, &UsageFilter::default());
    assert_eq!(report.totals.ok, 1);
    assert_eq!(report.totals.refused, 1);
    assert_eq!(report.totals.errors, 1);
}

#[test]
fn render_mode_and_screenshot_are_counted() {
    let dir = TempDir::new("render");
    let ledger = UsageLedger::open(dir.path()).unwrap();
    ledger
        .record(
            "t",
            "k",
            "r1",
            1,
            RenderMode::Browser,
            true,
            Some("eu".into()),
            "ok",
            200,
            10,
            1,
        )
        .unwrap();
    ledger
        .record(
            "t",
            "k",
            "r2",
            1,
            RenderMode::None,
            false,
            None,
            "ok",
            200,
            10,
            1,
        )
        .unwrap();
    let entries = ledger.read_all().unwrap();
    let report = summarize(&entries, &UsageFilter::default());
    assert_eq!(report.totals.render_browser, 1);
    assert_eq!(report.totals.render_none, 1);
    assert_eq!(report.totals.screenshots, 1);
}

#[test]
fn filters_by_key_tenant_and_window() {
    let (_dir, entries) = ledger_with(&[
        (1, "k-1", 1, 1_000),
        (2, "k-1", 1, 5_000),
        (3, "k-2", 1, 9_000),
    ]);
    let by_key = UsageFilter {
        key_id: Some("k-1".into()),
        ..UsageFilter::default()
    };
    assert_eq!(summarize(&entries, &by_key).totals.requests, 2);

    let window = UsageFilter {
        since_ms: Some(4_000),
        until_ms: Some(8_000),
        ..UsageFilter::default()
    };
    assert_eq!(summarize(&entries, &window).totals.requests, 1);

    let tenant = UsageFilter {
        tenant_id: Some("nope".into()),
        ..UsageFilter::default()
    };
    assert_eq!(summarize(&entries, &tenant).totals.requests, 0);
    assert_eq!(summarize(&entries, &tenant).since_ms, 0);
}

#[test]
fn the_row_limit_caps_the_key_table() {
    let (_dir, entries) =
        ledger_with(&[(1, "a", 1, 1_000), (2, "b", 1, 2_000), (3, "c", 1, 3_000)]);
    let limited = UsageFilter {
        limit: 2,
        ..UsageFilter::default()
    };
    let report = summarize(&entries, &limited);
    assert_eq!(report.by_key.len(), 2);
    assert_eq!(report.totals.requests, 3, "the totals are not capped");
}

#[test]
fn per_day_series_is_oldest_first() {
    let (_dir, entries) =
        ledger_with(&[(1, "k", 1, 2 * DAY), (2, "k", 1, DAY), (3, "k", 1, 3 * DAY)]);
    let report = summarize(&entries, &UsageFilter::default());
    assert_eq!(report.by_day.len(), 3);
    assert!(report.by_day[0].day < report.by_day[1].day);
    assert_eq!(report.by_day[0].units, 1);
    assert_eq!(day_of(2 * DAY), "1970-01-03");
}

#[test]
fn monthly_units_rebuild_the_quota_counter() {
    let (_dir, entries) = ledger_with(&[
        (1, "k-1", 2, 1_000),
        (2, "k-1", 3, 2_000),
        (3, "k-2", 7, 3_000),
    ]);
    let month = month_of(1_000);
    assert_eq!(monthly_units(&entries, "k-1", month), 5);
    assert_eq!(monthly_units(&entries, "k-2", month), 7);
    assert_eq!(monthly_units(&entries, "k-3", month), 0);
    let by_key = monthly_units_by_key(&entries, month);
    assert_eq!(by_key.get("k-1"), Some(&5));
    assert_eq!(by_key.get("k-2"), Some(&7));
    assert_eq!(by_key.len(), 2);
}

#[test]
fn monthly_units_ignore_other_months() {
    let dir = TempDir::new("months");
    override_now(1_700_000_000_000);
    let ledger = UsageLedger::open(dir.path()).unwrap();
    record(&ledger, "k-1", 4, 1_700_000_000_000);
    override_now(1_700_000_000_000 + 40 * DAY);
    record(&ledger, "k-1", 6, 1_700_000_000_000 + 40 * DAY);
    clear_now_override();
    let entries = ledger.read_all().unwrap();
    let a = month_of(1_700_000_000_000);
    let b = month_of(1_700_000_000_000 + 40 * DAY);
    assert_eq!(monthly_units(&entries, "k-1", a), 4);
    assert_eq!(monthly_units(&entries, "k-1", b), 6);
}

// --------------------------------------------------------------- export

#[test]
fn csv_export_writes_rows_and_a_verifying_manifest() {
    let (dir, entries) = ledger_with(&[(1, "k-1", 1, 1_000), (2, "k-2", 2, 2_000)]);
    let out = export_csv(&entries, &UsageFilter::default(), dir.path(), "usage").unwrap();
    assert_eq!(out.rows, 2);
    assert!(!out.truncated);
    let text = std::fs::read_to_string(&out.path).unwrap();
    let lines: Vec<&str> = text.trim_end().lines().collect();
    assert_eq!(lines.len(), 3, "header plus two rows");
    assert!(lines[0].contains("key_id"));
    assert!(lines[1].contains("k-1"));

    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["rows"], 2);
    assert_eq!(manifest["truncated"], false);
    assert_eq!(manifest["max_rows"], serde_json::json!(EXPORT_MAX_ROWS));
    // The manifest's digest must be the digest of the payload on disk, so a
    // consumer can verify the file without trusting the header.
    let payload = std::fs::read(&out.path).unwrap();
    let mut h = sha2::Sha256::new();
    h.update(&payload);
    let mut digest = String::new();
    for b in h.finalize() {
        digest.push_str(&format!("{b:02x}"));
    }
    assert_eq!(manifest["sha256"], serde_json::json!(digest));
    assert_eq!(manifest["bytes"], serde_json::json!(payload.len()));
    assert_eq!(out.sha256, digest);
    assert_eq!(out.bytes, payload.len() as u64);
}

#[test]
fn jsonl_export_round_trips_through_the_ledger_reader() {
    let (dir, entries) = ledger_with(&[(1, "k-1", 1, 1_000), (2, "k-2", 2, 2_000)]);
    let out = export_jsonl(&entries, &UsageFilter::default(), dir.path(), "raw").unwrap();
    let text = std::fs::read_to_string(&out.path).unwrap();
    assert_eq!(text.lines().count(), 2);
    let parsed: Vec<LedgerEntry> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(parsed, entries);
}

#[test]
fn export_respects_the_filter_and_reports_truncation() {
    let (dir, entries) = ledger_with(&[
        (1, "k-1", 1, 1_000),
        (2, "k-2", 2, 2_000),
        (3, "k-3", 3, 3_000),
    ]);
    let filtered = UsageFilter {
        key_id: Some("k-1".into()),
        ..UsageFilter::default()
    };
    let out = export_csv(&entries, &filtered, dir.path(), "subset").unwrap();
    assert_eq!(out.rows, 1);
    assert!(!out.truncated);

    // The cap is a hard ceiling: one row over it is dropped and reported.
    let many: Vec<LedgerEntry> = (0..EXPORT_MAX_ROWS + 1)
        .map(|i| entry(i as u64 + 1, "k", 1, i as u64))
        .collect();
    let big = TempDir::new("cap");
    let out = export_csv(&many, &UsageFilter::default(), big.path(), "capped").unwrap();
    assert_eq!(out.rows, EXPORT_MAX_ROWS);
    assert!(out.truncated);
    let manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&out.manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["truncated"], true);
    assert_eq!(manifest["rows"], serde_json::json!(EXPORT_MAX_ROWS));
}

#[test]
fn export_sanitises_the_file_name() {
    let (dir, entries) = ledger_with(&[(1, "k", 1, 1_000)]);
    let out = export_csv(
        &entries,
        &UsageFilter::default(),
        dir.path(),
        "../evil name",
    )
    .unwrap();
    let name = out
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap()
        .to_string();
    assert!(
        !name.contains('/'),
        "the name must stay inside the directory"
    );
    assert!(!name.contains(".."), "the name must not traverse");
    assert!(!name.is_empty());
}

#[test]
fn report_csv_export_writes_one_row_per_key() {
    let (dir, entries) = ledger_with(&[(1, "k-1", 1, 1_000), (2, "k-2", 2, 2_000)]);
    let report = summarize(&entries, &UsageFilter::default());
    let out = export_report_csv(&report, dir.path(), "report").unwrap();
    assert_eq!(out.rows, 2);
    let text = std::fs::read_to_string(&out.path).unwrap();
    assert_eq!(text.trim_end().lines().count(), 3);
}
