//! Bounded usage export.
//!
//! Follows the project's export conventions: the payload is written to a
//! private directory with `fsutil`, the row count is hard-capped before the
//! write starts, and a sibling manifest records what was written (rows,
//! bytes, sha256 of the payload) so a consumer can verify the file without
//! trusting the header.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use sha2::{Digest as _, Sha256};

use crate::usage::aggregate::{summarize, UsageFilter, UsageReport};
use crate::usage::ledger::LedgerEntry;

/// Hard cap on exported rows. A tenant asking for "all of last year" gets a
/// capped file and a manifest that says so, rather than a multi-gigabyte
/// download.
pub const EXPORT_MAX_ROWS: usize = 50_000;

/// One exported file plus its manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportResult {
    pub path: PathBuf,
    pub manifest_path: PathBuf,
    pub rows: usize,
    pub bytes: u64,
    pub sha256: String,
    pub truncated: bool,
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Export the ledger to CSV. `filter` selects the window; rows beyond
/// [`EXPORT_MAX_ROWS`] are dropped and reported in the manifest.
pub fn export_csv(
    entries: &[LedgerEntry],
    filter: &UsageFilter,
    dir: impl AsRef<Path>,
    name: &str,
) -> Result<ExportResult> {
    let dir = dir.as_ref();
    crate::fsutil::prepare_private_directory(dir)
        .with_context(|| format!("cannot prepare export directory {}", dir.display()))?;
    let safe = sanitize_name(name);
    let path = dir.join(format!("{safe}.csv"));

    let selected: Vec<&LedgerEntry> = filter.apply(entries.iter());
    let truncated = selected.len() > EXPORT_MAX_ROWS;
    let rows = selected.len().min(EXPORT_MAX_ROWS);

    let mut buf: Vec<u8> = Vec::with_capacity(rows * 256);
    {
        let mut w = csv::Writer::from_writer(&mut buf);
        w.write_record([
            "seq",
            "ts_ms",
            "tenant_id",
            "key_id",
            "request_id",
            "units",
            "render",
            "screenshot",
            "proxy_region",
            "status",
            "status_code",
            "bytes",
            "duration_ms",
        ])?;
        for e in selected.iter().take(rows) {
            w.write_record([
                e.seq.to_string(),
                e.ts_ms.to_string(),
                e.tenant_id.clone(),
                e.key_id.clone(),
                e.request_id.clone(),
                e.units.to_string(),
                e.render.as_str().to_string(),
                e.screenshot.to_string(),
                e.proxy_region.clone().unwrap_or_default(),
                e.status.clone(),
                e.status_code.to_string(),
                e.bytes.to_string(),
                e.duration_ms.to_string(),
            ])?;
        }
        w.flush()?;
    }

    write_payload(&path, &buf)?;
    let manifest = write_manifest(dir, &path, rows, truncated)?;
    Ok(ExportResult {
        path,
        manifest_path: manifest,
        rows,
        bytes: buf.len() as u64,
        sha256: sha256_hex(&buf),
        truncated,
    })
}

/// Export the ledger as JSON lines (one entry per line).
pub fn export_jsonl(
    entries: &[LedgerEntry],
    filter: &UsageFilter,
    dir: impl AsRef<Path>,
    name: &str,
) -> Result<ExportResult> {
    let dir = dir.as_ref();
    crate::fsutil::prepare_private_directory(dir)
        .with_context(|| format!("cannot prepare export directory {}", dir.display()))?;
    let safe = sanitize_name(name);
    let path = dir.join(format!("{safe}.jsonl"));

    let selected: Vec<&LedgerEntry> = filter.apply(entries.iter());
    let truncated = selected.len() > EXPORT_MAX_ROWS;
    let rows = selected.len().min(EXPORT_MAX_ROWS);

    let mut buf: Vec<u8> = Vec::with_capacity(rows * 512);
    for e in selected.iter().take(rows) {
        buf.extend_from_slice(serde_json::to_vec(e)?.as_slice());
        buf.push(b'\n');
    }

    write_payload(&path, &buf)?;
    let manifest = write_manifest(dir, &path, rows, truncated)?;
    Ok(ExportResult {
        path,
        manifest_path: manifest,
        rows,
        bytes: buf.len() as u64,
        sha256: sha256_hex(&buf),
        truncated,
    })
}

/// Export an aggregated report (per-key rows) as CSV.
pub fn export_report_csv(
    report: &UsageReport,
    dir: impl AsRef<Path>,
    name: &str,
) -> Result<ExportResult> {
    let dir = dir.as_ref();
    crate::fsutil::prepare_private_directory(dir)
        .with_context(|| format!("cannot prepare export directory {}", dir.display()))?;
    let safe = sanitize_name(name);
    let path = dir.join(format!("{safe}.csv"));

    let mut buf: Vec<u8> = Vec::with_capacity(report.by_key.len() * 256);
    {
        let mut w = csv::Writer::from_writer(&mut buf);
        w.write_record([
            "key_id",
            "tenant_id",
            "requests",
            "ok",
            "refused",
            "errors",
            "units",
            "bytes",
            "render_none",
            "render_browser",
            "screenshots",
            "first_ts_ms",
            "last_ts_ms",
        ])?;
        for r in &report.by_key {
            w.write_record([
                r.key_id.clone(),
                r.tenant_id.clone(),
                r.requests.to_string(),
                r.ok.to_string(),
                r.refused.to_string(),
                r.errors.to_string(),
                r.units.to_string(),
                r.bytes.to_string(),
                r.render_none.to_string(),
                r.render_browser.to_string(),
                r.screenshots.to_string(),
                r.first_ts_ms.to_string(),
                r.last_ts_ms.to_string(),
            ])?;
        }
        w.flush()?;
    }
    write_payload(&path, &buf)?;
    let manifest = write_manifest(dir, &path, report.by_key.len(), false)?;
    Ok(ExportResult {
        path,
        manifest_path: manifest,
        rows: report.by_key.len(),
        bytes: buf.len() as u64,
        sha256: sha256_hex(&buf),
        truncated: false,
    })
}

/// Render the report as a JSON document. Used by the export endpoint's
/// inline variant (small windows) and by the console's "export" action.
pub fn report_json(report: &UsageReport) -> serde_json::Value {
    serde_json::json!({
        "since_ms": report.since_ms,
        "until_ms": report.until_ms,
        "generated_at_ms": report.generated_at_ms,
        "key_count": report.key_count,
        "totals": report.totals,
        "by_key": report.by_key,
        "by_day": report.by_day,
    })
}

/// Build the report that `GET /v1/usage` serves.
pub fn build_report(entries: &[LedgerEntry], filter: &UsageFilter) -> UsageReport {
    summarize(entries, filter)
}

fn write_payload(path: &Path, buf: &[u8]) -> Result<()> {
    crate::fsutil::write_private_atomic(path, buf)
        .with_context(|| format!("cannot write export {}", path.display()))?;
    Ok(())
}

fn write_manifest(dir: &Path, payload: &Path, rows: usize, truncated: bool) -> Result<PathBuf> {
    let manifest_path = dir.join(format!(
        "{}.manifest.json",
        payload
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("export")
    ));
    let payload_bytes = fs::read(payload)
        .with_context(|| format!("cannot read back export {}", payload.display()))?;
    let body = serde_json::json!({
        "schema": 1,
        "payload": payload.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        "rows": rows,
        "bytes": payload_bytes.len(),
        "sha256": sha256_hex(&payload_bytes),
        "truncated": truncated,
        "max_rows": EXPORT_MAX_ROWS,
    });
    crate::fsutil::write_private_atomic(&manifest_path, body.to_string().as_bytes())?;
    Ok(manifest_path)
}

/// Keep the exported file name inside the directory and free of path
/// separators: the name is operator input.
fn sanitize_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "usage".to_string()
    } else {
        cleaned.chars().take(64).collect()
    }
}
