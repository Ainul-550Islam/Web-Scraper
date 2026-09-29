//! Safe artifact delivery.
//!
//! Everything the control plane produces (a delivery's payload files, its
//! manifest, its checksums and its ZIP bundle; a crawl's run-output file)
//! is registered here and handed out **by registry id only**. A download
//! never takes a path from a client: the id must have been minted by this
//! process, the file it points at is re-canonicalized on every open, and it
//! must still live under the control root it was registered under.
//!
//! Guarantees, in the order they are checked:
//!
//! 1. **Identity** — the id parses as `art-<digits>` and was minted here.
//! 2. **State** — the artifact is finalized and its verification outcome is
//!    known. A file still being written, or one whose verification failed,
//!    is refused with 409 rather than served half-done.
//! 3. **Path safety** — the stored path is canonicalized and must still be
//!    inside the control root. Symlinks that move a file out of the root
//!    therefore fail closed.
//! 4. **Size** — the declared byte count is checked against the caller's cap
//!    before opening, and the real size is checked again after.
//! 5. **Content type / disposition** — derived from the file *name* we
//!    registered, and the filename in `Content-Disposition` is sanitized so
//!    a crafted name cannot inject headers or path components.
//!
//! Nothing in this module reads a file the server did not write, and no
//! response contains a filesystem path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::api::epoch_secs;
use crate::control::{ArtifactId, ControlError};

/// How many artifacts the registry retains. Oldest finalized entries are
/// evicted first; entries still being written are never evicted.
pub const MAX_ARTIFACTS: usize = 256;

/// Why an artifact is not currently downloadable (or is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactState {
    /// Registered, still being written by its job. Not downloadable.
    Writing,
    /// Written, size and sha256 recorded, verification outcome known.
    /// Downloadable.
    Ready,
    /// The producing job failed, or verification failed. Never downloadable;
    /// the audit trail still points at it.
    Failed,
}

impl ArtifactState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Writing => "writing",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }
    pub fn downloadable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// What the artifact is for. Purely descriptive, but it is what lets the
/// console put `manifest.json` and `records.csv` in different columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactRole {
    /// A data payload (records.csv, records.xlsx, dataset-x.parquet, …).
    Payload,
    /// `manifest.json` (and its detached signature).
    Manifest,
    /// `checksums.txt`.
    Checksums,
    /// The ZIP bundle.
    Bundle,
    /// The run-output file a crawl wrote.
    RunOutput,
}

impl ArtifactRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Payload => "payload",
            Self::Manifest => "manifest",
            Self::Checksums => "checksums",
            Self::Bundle => "bundle",
            Self::RunOutput => "run_output",
        }
    }
}

/// The verification outcome carried with an artifact. `"not_requested"` is a
/// known state, not a gap: an export submitted with `verify: false` says so
/// instead of wearing a badge it has not earned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    NotRequested,
    Ok {
        records: u64,
        records_basis: String,
        files: usize,
    },
    Failed {
        reason: String,
    },
}

impl VerificationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRequested => "not_requested",
            Self::Ok { .. } => "ok",
            Self::Failed { .. } => "failed",
        }
    }
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::NotRequested => serde_json::json!({ "state": "not_requested" }),
            Self::Ok {
                records,
                records_basis,
                files,
            } => serde_json::json!({
                "state": "ok", "records": records,
                "records_basis": records_basis, "files": files,
            }),
            Self::Failed { reason } => serde_json::json!({ "state": "failed", "reason": reason }),
        }
    }
}

/// Public artifact metadata. The stored path is `#[serde(skip)]`: it is
/// server-side bookkeeping and must never cross the wire.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub id: String,
    /// Producing control job, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// File name as registered (sanitized again for `Content-Disposition`).
    pub name: String,
    pub role: ArtifactRole,
    pub content_type: &'static str,
    pub bytes: u64,
    /// sha256 of the finalized file; empty while still writing.
    pub sha256: String,
    pub state: ArtifactState,
    /// Free-text for a failed artifact: a reason, never a path, never a
    /// secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    pub verification: VerificationState,
    pub created_ts: u64,
    #[serde(skip)]
    path: PathBuf,
}

impl ArtifactRecord {
    /// Console-facing JSON: everything except the path.
    pub fn to_json(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(self).unwrap_or(serde_json::Value::Null);
        if let Some(obj) = v.as_object_mut() {
            obj.insert("state".to_string(), self.state.as_str().into());
            obj.insert("role".to_string(), self.role.as_str().into());
            obj.insert("downloadable".to_string(), self.state.downloadable().into());
            // The derived enum form (`{"ok": {...}}`) is awkward for clients;
            // the tagged shape is the contract.
            obj.insert("verification".to_string(), self.verification.to_json());
        }
        v
    }
}

/// MIME type for a registered file name. Unknown extensions get
/// `application/octet-stream` — never text, which a browser might execute or
/// render under our origin.
pub fn content_type_for(name: &str) -> &'static str {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "csv" => "text/csv; charset=utf-8",
        "jsonl" | "ndjson" => "application/x-ndjson",
        "json" => "application/json",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pdf" => "application/pdf",
        "parquet" => "application/vnd.apache.parquet",
        "xml" => "application/xml",
        "zip" => "application/zip",
        "txt" => "text/plain; charset=utf-8",
        "sig" | "sha256" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Strip anything that could break out of a `Content-Disposition` header:
/// directory components, quotes, backslashes, CR/LF and control characters.
/// A name that sanitizes to nothing becomes `artifact.bin`.
pub fn safe_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| {
            !matches!(c, '"' | '\\' | '/' | '\r' | '\n' | '\0')
                && !((*c as u32) < 0x20 || *c as u32 == 0x7f)
        })
        .collect();
    let cleaned = cleaned.trim().to_string();
    if cleaned.is_empty() {
        "artifact.bin".to_string()
    } else {
        cleaned
    }
}

/// `attachment; filename="…"` — the only form we emit, so a download never
/// renders inline under the console's origin.
pub fn content_disposition(name: &str) -> String {
    format!("attachment; filename=\"{}\"", safe_filename(name))
}

/// Why a download was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadErrorKind {
    /// No such artifact (unknown id, or it was evicted).
    NotFound,
    /// Finalized but not downloadable (still writing, or verification failed).
    NotReady,
    /// Larger than the caller's cap.
    TooLarge,
    /// The stored path escaped the control root, or vanished, or is no longer
    /// a regular file. Fail closed; the answer is never "serve it anyway".
    PathViolation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadError {
    pub kind: DownloadErrorKind,
    pub message: String,
}

impl DownloadError {
    fn not_found(id: &str) -> Self {
        Self {
            kind: DownloadErrorKind::NotFound,
            message: format!("no artifact `{id}`"),
        }
    }
    fn not_ready(state: ArtifactState, failure: Option<&str>) -> Self {
        let base = match state {
            ArtifactState::Writing => "artifact is still being written",
            ArtifactState::Failed => "artifact failed verification or its job failed",
            ArtifactState::Ready => "artifact is not downloadable",
        };
        Self {
            kind: DownloadErrorKind::NotReady,
            message: match failure {
                Some(f) => format!("{base}: {f}"),
                None => base.to_string(),
            },
        }
    }
    fn too_large(bytes: u64, cap: u64) -> Self {
        Self {
            kind: DownloadErrorKind::TooLarge,
            message: format!("artifact is {bytes} bytes, over the {cap} byte download cap"),
        }
    }
    fn path_violation() -> Self {
        Self {
            kind: DownloadErrorKind::PathViolation,
            message: "artifact is no longer inside the control root".to_string(),
        }
    }

    pub fn to_control_error(&self) -> ControlError {
        match self.kind {
            DownloadErrorKind::NotFound => ControlError::not_found(&self.message),
            DownloadErrorKind::NotReady | DownloadErrorKind::PathViolation => {
                ControlError::conflict(&self.message)
            }
            DownloadErrorKind::TooLarge => ControlError::invalid(&self.message),
        }
    }

    pub fn status(&self) -> u16 {
        match self.kind {
            DownloadErrorKind::NotFound => 404,
            DownloadErrorKind::NotReady | DownloadErrorKind::PathViolation => 409,
            DownloadErrorKind::TooLarge => 413,
        }
    }
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// A downloaded artifact, already size-checked and path-checked.
#[derive(Debug, Clone)]
pub struct OpenedArtifact {
    pub record: ArtifactRecord,
    path: PathBuf,
}

impl OpenedArtifact {
    /// Read the whole file. Safe because `open` already verified the size
    /// against the cap and the path against the root.
    pub fn read(&self) -> Result<Vec<u8>, DownloadError> {
        std::fs::read(&self.path).map_err(|_| DownloadError::path_violation())
    }
    pub fn content_type(&self) -> &'static str {
        self.record.content_type
    }
    pub fn content_disposition(&self) -> String {
        content_disposition(&self.record.name)
    }
}

/// Bounded registry of downloadable artifacts.
pub struct ArtifactStore {
    /// Every registered path must live under this directory (canonical).
    root: PathBuf,
    inner: Mutex<BTreeMap<String, ArtifactRecord>>,
}

impl ArtifactStore {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn under_root(&self, p: &Path) -> bool {
        match std::fs::canonicalize(p) {
            Ok(c) => c.starts_with(&self.root),
            Err(_) => false,
        }
    }

    /// Register a file that already exists on disk. The path is canonicalized
    /// and must be inside the control root; a file outside it is refused
    /// rather than adopted.
    pub fn register(
        &self,
        path: PathBuf,
        name: String,
        role: ArtifactRole,
        job: Option<String>,
        verification: VerificationState,
    ) -> Result<ArtifactId, ControlError> {
        if !self.under_root(&path) {
            return Err(ControlError::conflict(
                "artifact path is outside the control root",
            ));
        }
        let bytes = std::fs::metadata(&path)
            .map(|m| m.len())
            .map_err(|_| ControlError::conflict("artifact file is unreadable"))?;
        let id = ArtifactId::next();
        let content_type = content_type_for(&name);
        let rec = ArtifactRecord {
            id: id.as_str().to_string(),
            job,
            name,
            role,
            content_type,
            bytes,
            sha256: String::new(),
            state: ArtifactState::Writing,
            failure: None,
            verification,
            created_ts: epoch_secs(),
            path,
        };
        self.evict_if_needed();
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.as_str().to_string(), rec);
        Ok(id)
    }

    /// Mark an artifact written: record its real size and sha256. The state
    /// only becomes `Ready` when the verification outcome is known (which
    /// [`ArtifactStore::mark_verified`] or a `NotRequested` registration
    /// already supplies).
    pub fn finalize(&self, id: &str, sha256: String) -> Result<(), ControlError> {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(rec) = g.get_mut(id) else {
            return Err(ControlError::not_found(format!("no artifact `{id}`")));
        };
        if rec.state == ArtifactState::Failed {
            return Ok(());
        }
        let bytes = std::fs::metadata(&rec.path)
            .map(|m| m.len())
            .map_err(|_| ControlError::conflict("artifact file vanished before finalizing"))?;
        if !self.under_root(&rec.path) {
            rec.state = ArtifactState::Failed;
            rec.failure = Some("artifact left the control root".to_string());
            return Err(ControlError::conflict(
                "artifact path is outside the control root",
            ));
        }
        rec.bytes = bytes;
        rec.sha256 = sha256;
        rec.state = ArtifactState::Ready;
        Ok(())
    }

    /// Record a verification outcome. A failure moves the artifact to
    /// `Failed`, which is what disables the download button.
    pub fn mark_verified(&self, id: &str, v: VerificationState) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(rec) = g.get_mut(id) {
            rec.verification = v.clone();
            if let VerificationState::Failed { reason } = &v {
                rec.state = ArtifactState::Failed;
                rec.failure = Some(reason.clone());
            }
        }
    }

    /// Mark an artifact (and optionally every sibling of its job) failed.
    pub fn mark_failed(&self, id: &str, reason: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(rec) = g.get_mut(id) {
            rec.state = ArtifactState::Failed;
            rec.failure = Some(reason.to_string());
        }
    }

    /// Fail every artifact still in `Writing` for a job whose run failed.
    pub fn fail_job_artifacts(&self, job: &str, reason: &str) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        for rec in g.values_mut() {
            if rec.job.as_deref() == Some(job) && rec.state == ArtifactState::Writing {
                rec.state = ArtifactState::Failed;
                rec.failure = Some(reason.to_string());
            }
        }
    }

    pub fn get(&self, id: &str) -> Option<ArtifactRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }

    /// Artifact ids belonging to a job, newest last.
    pub fn for_job(&self, job: &str) -> Vec<ArtifactRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|r| r.job.as_deref() == Some(job))
            .cloned()
            .collect()
    }

    pub fn list(&self) -> Vec<ArtifactRecord> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Open an artifact for download, applying every guard in order.
    /// `max_bytes` is the caller's cap: the API passes
    /// [`crate::control::MAX_DOWNLOAD_BYTES`], tests pass smaller ones to
    /// exercise the limit without writing a huge file.
    pub fn open_bounded(&self, id: &str, max_bytes: u64) -> Result<OpenedArtifact, DownloadError> {
        let rec = {
            let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
            g.get(id).cloned()
        };
        let Some(rec) = rec else {
            return Err(DownloadError::not_found(id));
        };
        if !rec.state.downloadable() {
            return Err(DownloadError::not_ready(rec.state, rec.failure.as_deref()));
        }
        if rec.bytes > max_bytes {
            return Err(DownloadError::too_large(rec.bytes, max_bytes));
        }
        if !self.under_root(&rec.path) {
            return Err(DownloadError::path_violation());
        }
        let meta = std::fs::metadata(&rec.path).map_err(|_| DownloadError::path_violation())?;
        if !meta.is_file() {
            return Err(DownloadError::path_violation());
        }
        if meta.len() > max_bytes {
            return Err(DownloadError::too_large(meta.len(), max_bytes));
        }
        let path = std::fs::canonicalize(&rec.path).map_err(|_| DownloadError::path_violation())?;
        if !path.starts_with(&self.root) {
            return Err(DownloadError::path_violation());
        }
        Ok(OpenedArtifact { record: rec, path })
    }

    pub fn open(&self, id: &str) -> Result<OpenedArtifact, DownloadError> {
        self.open_bounded(id, crate::control::MAX_DOWNLOAD_BYTES)
    }

    /// Eviction: drop the oldest `Ready`/`Failed` entries when at capacity.
    /// Artifacts still being written are never evicted from under a job.
    fn evict_if_needed(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if g.len() < MAX_ARTIFACTS {
            return;
        }
        let mut victims: Vec<(u64, String)> = g
            .values()
            .filter(|r| r.state != ArtifactState::Writing)
            .map(|r| (r.created_ts, r.id.clone()))
            .collect();
        victims.sort();
        for (_, id) in victims {
            if g.len() < MAX_ARTIFACTS {
                break;
            }
            g.remove(&id);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests;
