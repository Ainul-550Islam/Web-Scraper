//! Read-only repository abstraction over the engine's operational files.
//!
//! The engine persists operational history as JSONL: run history
//! (`schedule.history_path`, surfaced by `GET /runs` and `harvest history`),
//! the audit log (`audit_path`), and sink output. This module provides the
//! ONE shared way for read-side features (search, tooling, tests) to pull
//! bounded windows out of those files — without introducing a second storage
//! system, without mutating anything, and without unbounded reads:
//!
//! * files above [`MAX_SCAN_BYTES`] are read from the tail in chunks, so a
//!   10 GB log never loads into memory to answer "last 50 rows";
//! * at most [`MAX_RECORDS_PER_QUERY`] records are ever materialized;
//! * malformed lines are counted, never fatal (a torn final line written
//!   mid-crash must not brick history viewers).

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Hard cap on bytes read per query (tail-chunked). 4 MiB is already generous
/// for "the last N records" of a JSONL log.
pub const MAX_SCAN_BYTES: u64 = 4 * 1024 * 1024;
/// Hard cap on materialized records per query.
pub const MAX_RECORDS_PER_QUERY: usize = 512;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanStats {
    pub lines_scanned: usize,
    pub malformed_lines: usize,
    pub bytes_read: u64,
    /// true when the file was longer than the budget and the oldest scanned
    /// records were NOT looked at (callers must surface this, not hide it)
    pub truncated: bool,
}

#[derive(Debug)]
pub struct Window {
    /// newest-first, as the history/audit convention in this repo (see
    /// `src/ops.rs` — tail is newest-first)
    pub records: Vec<Value>,
    pub stats: ScanStats,
    pub path: PathBuf,
}

/// Tail-read at most `want` newline-delimited JSON records from `path`.
/// Returns an empty window (not an error) when the file does not exist —
/// "no history yet" is a normal state; a read error (permissions, I/O) is
/// returned as `io::Error` so callers can tell the two apart.
pub fn read_tail(path: &Path, want: usize) -> std::io::Result<Window> {
    let want = want.clamp(1, MAX_RECORDS_PER_QUERY);
    let Ok(meta) = path.metadata() else {
        return Ok(Window {
            records: Vec::new(),
            stats: ScanStats::default(),
            path: path.to_path_buf(),
        });
    };
    let mut file = File::open(path)?;
    let size = meta.len();
    let budget = size.min(MAX_SCAN_BYTES);
    let truncated = size > MAX_SCAN_BYTES;
    let start = size - budget;
    file.seek(SeekFrom::Start(start))?;
    let mut raw = vec![0u8; budget as usize];
    file.read_exact(&mut raw)?;
    if start > 0 {
        // drop a possibly-partial first line
        if let Some(nl) = raw.iter().position(|b| *b == b'\n') {
            raw.drain(..=nl);
        }
    }
    let mut records = Vec::with_capacity(want.min(64));
    let mut stats = ScanStats {
        bytes_read: budget,
        truncated,
        ..Default::default()
    };
    // walk lines from the END so "newest-first, want N" costs O(N) parses
    let text = String::from_utf8_lossy(&raw);
    let lines: Vec<&str> = text.lines().collect();
    stats.lines_scanned = lines.len();
    for line in lines.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) => {
                records.push(v);
                if records.len() >= want {
                    break;
                }
            }
            Err(_) => stats.malformed_lines += 1,
        }
    }
    Ok(Window {
        records,
        stats,
        path: path.to_path_buf(),
    })
}

/// The run-history file of a config (schedule.history_path).
pub fn history_path(cfg: &crate::config::Config) -> PathBuf {
    cfg.schedule.history_path.clone()
}

/// The audit file path of a config, if auditing is on.
pub fn audit_path(cfg: &crate::config::Config) -> Option<PathBuf> {
    cfg.audit_path.clone()
}

/// Read a run-history window.
pub fn history_records(cfg: &crate::config::Config, want: usize) -> std::io::Result<Window> {
    read_tail(&history_path(cfg), want)
}

/// Read the engine EVENT LOG window (config `events_path`, JSONL — written by
/// `harvest::runtime::events_log_writer`). Not configured ⇒ empty window:
/// disabled is not broken (health doctrine).
pub fn events_records(cfg: &crate::config::Config, want: usize) -> std::io::Result<Window> {
    match cfg.events_path.as_deref() {
        None => Ok(Window {
            records: Vec::new(),
            stats: ScanStats::default(),
            path: PathBuf::new(),
        }),
        Some(p) => read_tail(p, want),
    }
}

/// The audit REPORT is a single pretty-printed JSON document (atomic
/// final write — see `src/audit.rs`), not JSONL. `Ok(None)` when auditing
/// is off or the file does not exist yet; malformed bytes are an Err.
pub fn audit_report(cfg: &crate::config::Config) -> std::io::Result<Option<Value>> {
    let Some(path) = audit_path(cfg) else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let meta = path.metadata()?;
    if meta.len() > MAX_SCAN_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("audit report larger than the {MAX_SCAN_BYTES} byte read budget"),
        ));
    }
    let raw = std::fs::read_to_string(&path)?;
    let v: Value = serde_json::from_str(&raw)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpdir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "harvest-repo-{}-{}-{}",
            tag,
            std::process::id(),
            uuid_like()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
    fn uuid_like() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    }

    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tail_is_newest_first_and_count_bounded() {
        let d = tmpdir("tail");
        let f = d.join("h.jsonl");
        {
            let mut w = File::create(&f).unwrap();
            for i in 0..20 {
                writeln!(w, "{{\"n\":{i}}}").unwrap();
            }
        }
        let _c = Cleanup(d);
        let win = read_tail(&f, 5).unwrap();
        assert_eq!(win.records.len(), 5);
        assert_eq!(win.records[0]["n"], Value::from(19), "newest first");
        assert_eq!(win.records[4]["n"], Value::from(15));
        assert_eq!(win.stats.lines_scanned, 20);
        assert_eq!(win.stats.malformed_lines, 0);
        assert!(!win.stats.truncated);
    }

    #[test]
    fn missing_file_is_an_empty_window_not_an_error() {
        let d = tmpdir("missing");
        let _c = Cleanup(d.clone());
        let win = read_tail(&d.join("nope.jsonl"), 10).unwrap();
        assert!(win.records.is_empty());
        assert_eq!(win.stats, ScanStats::default());
    }

    #[test]
    fn torn_and_malformed_lines_are_counted_not_fatal() {
        let d = tmpdir("torn");
        let f = d.join("h.jsonl");
        {
            let mut w = File::create(&f).unwrap();
            writeln!(w, "{{\"n\":1}}").unwrap();
            w.write_all(b"{\"broken\"\n").unwrap();
            w.write_all(b"{\"n\":2}").unwrap(); // no trailing newline (torn)
        }
        let _c = Cleanup(d);
        let win = read_tail(&f, 10).unwrap();
        // broken line counted; torn-but-complete JSON still parseable
        assert_eq!(win.stats.malformed_lines, 1);
        let ns: Vec<i64> = win.records.iter().filter_map(|r| r["n"].as_i64()).collect();
        assert!(
            ns.contains(&1) && ns.contains(&2),
            "both intact rows found: {ns:?}"
        );
    }

    #[test]
    fn huge_files_are_tail_chunked_with_a_honest_truncation_flag() {
        let d = tmpdir("big");
        let f = d.join("h.jsonl");
        {
            let mut w = File::create(&f).unwrap();
            let pad = "x".repeat(1024);
            // ~5 MiB, over MAX_SCAN_BYTES
            for i in 0..5200 {
                writeln!(w, "{{\"n\":{i},\"pad\":\"{pad}\"}}").unwrap();
            }
        }
        let _c = Cleanup(d);
        let size = f.metadata().unwrap().len();
        assert!(size > MAX_SCAN_BYTES, "fixture must exceed the budget");
        let win = read_tail(&f, 3).unwrap();
        assert!(
            win.stats.truncated,
            "reader must admit it only saw the tail"
        );
        assert_eq!(win.records.len(), 3);
        assert_eq!(win.records[0]["n"], Value::from(5199), "newest tail intact");
        assert!(win.stats.bytes_read <= MAX_SCAN_BYTES, "budget honored");
    }

    #[test]
    fn want_is_clamped_to_the_hard_cap() {
        let d = tmpdir("clamp");
        let f = d.join("h.jsonl");
        {
            let mut w = File::create(&f).unwrap();
            for i in 0..(MAX_RECORDS_PER_QUERY + 10) {
                writeln!(w, "{{\"n\":{i}}}").unwrap();
            }
        }
        let _c = Cleanup(d);
        let win = read_tail(&f, usize::MAX).unwrap();
        assert_eq!(
            win.records.len(),
            MAX_RECORDS_PER_QUERY,
            "one query can never materialize more than the cap"
        );
    }
}
