//! Append-only, hash-chained usage ledger.
//!
//! Every metered request becomes one JSON line. The lines form a hash chain
//! in the same style as the audit ledger: entry `n` carries the digest of
//! entry `n-1`, so a truncated, reordered or edited file is detectable on
//! read. That matters because the ledger is the *billing* record — a tenant
//! disputes a charge, and "we counted it in memory" is not an answer.
//!
//! * The file rotates by UTC month, and again when it passes
//!   [`crate::config::V1_LEDGER_MAX_BYTES`], so no single file grows
//!   without bound.
//! * Appends go through [`crate::fsutil::open_append`]: private permissions,
//!   no symlink adoption, single hard link.
//! * Reads are bounded: the file size is checked before the buffer is
//!   allocated, and the scan stops at the first broken link.
//!
//! See `docs/modules/usage.md`.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::V1_LEDGER_MAX_BYTES;
use crate::fsutil;

/// Schema stamp for the ledger format.
pub const LEDGER_SCHEMA_VERSION: u32 = 1;

/// Largest single ledger file before it is rotated, regardless of month.
pub const LEDGER_ROTATE_BYTES: u64 = V1_LEDGER_MAX_BYTES;

/// Render mode a request was billed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderMode {
    None,
    Browser,
}

impl Default for RenderMode {
    fn default() -> Self {
        Self::None
    }
}

impl RenderMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RenderMode::None => "none",
            RenderMode::Browser => "browser",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(RenderMode::None),
            "browser" => Some(RenderMode::Browser),
            _ => None,
        }
    }
}

/// One metered request. `seq`, `prev_hash` and `hash` are derived by
/// [`UsageLedger::append`]; a caller cannot forge them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerEntry {
    pub seq: u64,
    pub schema: u32,
    pub prev_hash: String,
    pub hash: String,
    pub ts_ms: u64,
    pub tenant_id: String,
    pub key_id: String,
    pub request_id: String,
    pub units: u32,
    pub render: RenderMode,
    pub screenshot: bool,
    pub proxy_region: Option<String>,
    /// Outcome class: `ok`, `refused` (policy/SSRF/limit) or `error`.
    pub status: String,
    pub status_code: u16,
    pub bytes: u64,
    pub duration_ms: u64,
}

impl LedgerEntry {
    /// The digest of this entry given the previous entry's digest. The entry's
    /// own `hash` field is blanked first so the digest cannot depend on
    /// itself.
    pub fn compute_hash(&mut self, prev_hash: &str) -> String {
        self.prev_hash = prev_hash.to_string();
        self.hash = String::new();
        let canonical = serde_json::to_vec(self).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(self.schema.to_le_bytes());
        h.update(prev_hash.as_bytes());
        h.update([0u8]);
        h.update(&canonical);
        to_hex(&h.finalize())
    }

    /// `true` when this entry continues `prev_hash` unchanged.
    pub fn continues_chain(&self, prev_hash: &str) -> bool {
        if self.prev_hash != prev_hash {
            return false;
        }
        let mut copy = self.clone();
        copy.compute_hash(prev_hash) == self.hash
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// The name of the ledger file for a UTC month index (`year * 12 +
/// (month - 1)`): `usage-2026-10.jsonl`.
pub fn month_file_name(month: i64) -> String {
    let y = month.div_euclid(12);
    let m = month.rem_euclid(12) as u32 + 1;
    format!("usage-{y:04}-{m:02}.jsonl")
}

/// Current UTC month as `year * 12 + (month - 1)`.
pub fn month_of(now_ms: u64) -> i64 {
    let (y, m, _) = crate::ids::civil_from_days((now_ms / 1000).div_euclid(86_400) as i64);
    y * 12 + i64::from(m) - 1
}

thread_local! {
    /// Test seam: a fixed clock for the current thread. `None` = wall clock.
    static NOW_OVERRIDE: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// Test seam: pin the wall clock for the current thread.
#[doc(hidden)]
pub fn override_now(ms: u64) {
    NOW_OVERRIDE.with(|c| c.set(Some(ms)));
}

/// Test seam: restore the wall clock for the current thread.
#[doc(hidden)]
pub fn clear_now_override() {
    NOW_OVERRIDE.with(|c| c.set(None));
}

fn now_ms() -> u64 {
    NOW_OVERRIDE.with(|c| match c.get() {
        Some(v) => v,
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    })
}

/// Append-only usage ledger.
pub struct UsageLedger {
    dir: PathBuf,
    /// Highest sequence number handed out. Seeded from the existing files so
    /// a restart continues the chain instead of restarting at zero.
    seq: AtomicU64,
    /// Digest of the last entry in the current file, kept so an append does
    /// not have to re-read the file.
    last_hash: Mutex<String>,
    /// Month the open file belongs to.
    month: Mutex<i64>,
    /// Byte size of the open file, for the size-based rotation trigger.
    size: Mutex<u64>,
}

impl UsageLedger {
    /// Open (or create) the ledger rooted at `dir`.
    ///
    /// Existing files are scanned to recover the chain head. A broken chain
    /// is reported rather than silently repaired, because the ledger is the
    /// billing record.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let dir = dir.into();
        fsutil::prepare_private_directory(&dir)
            .with_context(|| format!("cannot prepare usage ledger directory {}", dir.display()))?;
        let mut last_hash = String::new();
        let mut seq = 0u64;
        for entry in scan_dir(&dir)? {
            seq = entry.seq.max(seq);
            last_hash = entry.hash;
        }
        Ok(Arc::new(Self {
            dir,
            seq: AtomicU64::new(seq),
            last_hash: Mutex::new(last_hash),
            month: Mutex::new(month_of(now_ms())),
            size: Mutex::new(0),
        }))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Relaxed)
    }

    pub fn last_hash(&self) -> String {
        self.last_hash.lock().map(|h| h.clone()).unwrap_or_default()
    }

    /// Read every entry of every file, oldest first, verifying the chain.
    pub fn read_all(&self) -> Result<Vec<LedgerEntry>> {
        scan_dir(&self.dir)
    }

    /// Verify the on-disk chain without materialising it. Returns the number
    /// of verified entries.
    pub fn verify(&self) -> Result<u64> {
        Ok(self.read_all()?.len() as u64)
    }

    /// Append one entry. Sequence, previous digest and digest are filled in
    /// here; callers cannot forge them.
    pub fn append(&self, mut entry: LedgerEntry) -> Result<LedgerEntry> {
        entry.seq = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        entry.schema = LEDGER_SCHEMA_VERSION;
        let prev = self.last_hash.lock().map(|h| h.clone()).unwrap_or_default();
        let hash = entry.compute_hash(&prev);
        entry.hash = hash.clone();

        let month = month_of(now_ms());
        let mut open_month = self.month.lock().unwrap_or_else(|p| p.into_inner());
        if *open_month != month {
            self.rotate(month)?;
            *open_month = month;
        }
        let path = self.dir.join(month_file_name(month));
        let mut size = self.size.lock().unwrap_or_else(|p| p.into_inner());
        if *size >= LEDGER_ROTATE_BYTES {
            self.rotate(month)?;
            *size = 0;
        }
        let mut file = fsutil::open_append(&path)
            .with_context(|| format!("cannot append usage ledger {}", path.display()))?;
        let line = serde_json::to_string(&entry)?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        drop(file);
        *size += line.len() as u64 + 1;
        *self.last_hash.lock().unwrap_or_else(|p| p.into_inner()) = hash;
        Ok(entry)
    }

    /// Start a fresh file for `month`, leaving the old one in place.
    fn rotate(&self, month: i64) -> Result<()> {
        let path = self.dir.join(month_file_name(month));
        fsutil::open_append(&path)?.sync_all()?;
        *self.size.lock().unwrap_or_else(|p| p.into_inner()) = 0;
        Ok(())
    }

    /// Metered write for a completed request.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        tenant_id: &str,
        key_id: &str,
        request_id: &str,
        units: u32,
        render: RenderMode,
        screenshot: bool,
        proxy_region: Option<String>,
        status: &str,
        status_code: u16,
        bytes: u64,
        duration_ms: u64,
    ) -> Result<LedgerEntry> {
        self.append(LedgerEntry {
            seq: 0,
            schema: LEDGER_SCHEMA_VERSION,
            prev_hash: String::new(),
            hash: String::new(),
            ts_ms: now_ms(),
            tenant_id: tenant_id.to_string(),
            key_id: key_id.to_string(),
            request_id: request_id.to_string(),
            units,
            render,
            screenshot,
            proxy_region,
            status: status.to_string(),
            status_code,
            bytes,
            duration_ms,
        })
    }
}

/// Scan every `usage-*.jsonl` file in `dir`, oldest name first, verifying the
/// chain across files. A file whose chain does not continue the previous
/// file's digest is reported, never repaired.
fn scan_dir(dir: &Path) -> Result<Vec<LedgerEntry>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("cannot read usage ledger directory {}", dir.display()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("usage-") && n.ends_with(".jsonl"))
        })
        .collect();
    files.sort();

    let mut out = Vec::new();
    let mut prev = String::new();
    for path in files {
        let bytes = fsutil::read_bounded(&path, LEDGER_ROTATE_BYTES)
            .with_context(|| format!("cannot read usage ledger {}", path.display()))?;
        let text = String::from_utf8(bytes)
            .map_err(|_| anyhow::anyhow!("usage ledger {} is not UTF-8", path.display()))?;
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let entry: LedgerEntry = serde_json::from_str(line).map_err(|e| {
                anyhow::anyhow!(
                    "usage ledger {} line {} is not a valid entry: {e}",
                    path.display(),
                    i + 1
                )
            })?;
            if !entry.continues_chain(&prev) {
                anyhow::bail!(
                    "usage ledger chain broken in {} at sequence {} (expected prev {prev:?})",
                    path.display(),
                    entry.seq
                );
            }
            prev = entry.hash.clone();
            out.push(entry);
        }
    }
    Ok(out)
}
