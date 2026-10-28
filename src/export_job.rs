//! Export jobs: the existing exporters, wrapped in a bounded lifecycle.
//!
//! This module deliberately contains **no format code**. CSV, JSONL, XLSX,
//! PDF, Parquet, XML and the ZIP bundle are rendered by [`crate::export`];
//! the manifest and checksums are written there too; `harvest verify` is the
//! same [`crate::export::verify`] the CLI runs. What this file adds is the
//! control-plane *contract* around one invocation:
//!
//! - where the input comes from (only files the attached configuration
//!   already declares — never a client-supplied path),
//! - how many records may be rendered (`max_records`, enforced by truncation
//!   *before* the exporter runs, not by hoping it stops),
//! - how big the delivery may get (`max_export_bytes`, checked against the
//!   real bytes on disk afterwards),
//! - whether it must pass verification before any artifact is downloadable,
//! - and what to register as artifacts with [`crate::download`].
//!
//! It is a pure function plus bookkeeping: `run` is synchronous and has no
//! dependency on the scheduler, so it can be executed from a worker task, a
//! test, or the CLI-equivalent without any of them growing a second path.

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{Config, SinkCfg};
use crate::control::{CommandLimits, ControlError, ExportRequest};
use crate::download::{ArtifactRole, VerificationState};
use crate::export::{ExportFormat, ExportOpts};

/// Upper bound on source files an export job will read from a partitioned
/// dataset directory.
pub const MAX_SOURCE_FILES: usize = 64;

/// The run's declared output files, as the export job's only permitted
/// source. A client cannot name a path here: the source is whatever the
/// attached configuration says the run writes.
pub fn source_files(cfg: &Config) -> Result<Vec<PathBuf>, ControlError> {
    match &cfg.output {
        SinkCfg::Jsonl { path } | SinkCfg::Csv { path } | SinkCfg::Sqlite { path, .. } => {
            if !path.exists() {
                return Err(ControlError::unavailable(
                    "the configured output file does not exist yet; run a crawl first",
                ));
            }
            Ok(vec![path.clone()])
        }
        SinkCfg::Dataset { dir, .. } => {
            if !dir.exists() {
                return Err(ControlError::unavailable(
                    "the configured dataset directory does not exist yet; run a crawl first",
                ));
            }
            let files = crate::source_io::dataset_files(dir).map_err(|e| ControlError::unavailable(format!("cannot discover dataset parts: {e}")))?;
            if files.is_empty() { return Err(ControlError::unavailable("the configured dataset directory holds no .jsonl parts yet")); }
            Ok(files)
        }
        SinkCfg::Http { .. } | SinkCfg::Postgres { .. } | SinkCfg::Null {} => {
            Err(ControlError::unavailable(
                "the configured output sink is not a file this build can export from (use a jsonl, csv, sqlite or dataset sink)",
            ))
        }
    }
}

/// A validated export request, resolved against the command's limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportJobSpec {
    pub formats: Vec<ExportFormat>,
    pub dataset: Option<String>,
    pub bundle: bool,
    pub verify: bool,
    /// Effective record cap (already bounded by the command's limits).
    pub max_records: u64,
    pub max_export_bytes: u64,
}

impl ExportJobSpec {
    pub fn from_request(r: &ExportRequest, limits: &CommandLimits) -> Result<Self, ControlError> {
        let mut formats: Vec<ExportFormat> = Vec::new();
        for raw in &r.formats {
            let f = ExportFormat::parse(raw).ok_or_else(|| {
                ControlError::invalid(format!("unsupported export format `{raw}`"))
            })?;
            if !formats.contains(&f) {
                formats.push(f);
            }
        }
        if formats.is_empty() {
            return Err(ControlError::invalid(
                "`formats` must name at least one format",
            ));
        }
        if r.bundle && formats.len() == 1 && formats[0] == ExportFormat::Zip {
            return Err(ControlError::invalid(
                "`bundle` needs at least one payload format alongside zip",
            ));
        }
        // Bundle membership follows the CLI's rule: `zip` in the format list
        // (or an explicit `bundle: true`) means "bundle the rest".
        let bundle = if r.bundle && !formats.contains(&ExportFormat::Zip) {
            formats.push(ExportFormat::Zip);
            true
        } else {
            formats.contains(&ExportFormat::Zip)
        };
        let max_records = r
            .max_records
            .unwrap_or(limits.max_records)
            .min(limits.max_records);
        Ok(Self {
            formats,
            dataset: r.dataset.clone(),
            bundle,
            verify: r.verify,
            max_records,
            max_export_bytes: limits.max_export_bytes,
        })
    }

    /// Render order: the canonical pair first, then the adapters, then the
    /// bundle — matching `src/export.rs` so the manifest is byte-identical to
    /// the CLI's for the same inputs.
    pub fn format_names(&self) -> Vec<String> {
        self.formats
            .iter()
            .map(|f| f.as_str().to_string())
            .collect()
    }
}

/// One file a finished export produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProducedFile {
    pub path: PathBuf,
    pub name: String,
    pub role: ArtifactRole,
    pub bytes: u64,
    pub sha256: String,
}

/// What one export job produced.
#[derive(Debug, Clone, Serialize)]
pub struct ExportResult {
    pub records: u64,
    /// Unique (canonical) URLs the delivery covers.
    pub unique_urls: u64,
    pub formats: Vec<String>,
    pub files: Vec<ProducedFileName>,
    pub total_bytes: u64,
    pub verification: VerificationState,
    /// The directory the delivery lives in, relative to the control root —
    /// safe to show an operator, and never an absolute path.
    pub delivery: String,
}

/// Serializable twin of [`ProducedFile`] (no paths cross the API boundary).
#[derive(Debug, Clone, Serialize)]
pub struct ProducedFileName {
    pub name: String,
    pub role: String,
    pub bytes: u64,
    pub sha256: String,
}

/// Copy at most `max_records` JSONL lines out of `inputs` into `work_dir`.
/// The cap is applied *before* the exporter runs, so a job asked for 1,000
/// records cannot spend 5,000,000 records of I/O getting there.
pub fn truncate_inputs(
    inputs: &[PathBuf],
    work_dir: &Path,
    max_records: u64,
) -> Result<Vec<PathBuf>, String> {
    crate::source_io::stage(&Config::default(), inputs, work_dir, max_records)
        .map_err(|e| e.to_string())
}

fn hash_output_file(path: &Path, cap: u64) -> std::io::Result<(String, u64)> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "output needs a filename")
    })?;
    let mut file = crate::fsutil::open_within(parent, &parent.join(name), false)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "produced output is not a regular file",
        ));
    }
    if metadata.len() > cap {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "produced output exceeds its byte cap",
        ));
    }
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65_536];
    loop {
        crate::work_cancel::check()?;
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes = bytes.saturating_add(read as u64);
        if bytes > cap || bytes > metadata.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "produced output changed or grew beyond its pinned size",
            ));
        }
        hasher.update(&buffer[..read]);
    }
    if bytes != metadata.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "produced output changed or shrank while being hashed",
        ));
    }
    Ok((format!("{:x}", hasher.finalize()), bytes))
}

/// Run one export with the existing exporter and register nothing. Returns
/// the produced files (with hashes) plus the verification outcome; the caller
/// (the executor) owns artifact registration and audit.
pub fn run(
    spec: &ExportJobSpec,
    cfg: &Config,
    out_dir: &Path,
) -> Result<(ExportResult, Vec<ProducedFile>), String> {
    let inputs = source_files(cfg).map_err(|e| e.message)?;
    std::fs::create_dir_all(out_dir).map_err(|e| format!("cannot create the delivery dir: {e}"))?;
    let label = out_dir
        .file_name()
        .ok_or("delivery directory needs a name")?
        .to_string_lossy();
    let stage = out_dir.with_file_name(format!(".source-stage-{label}"));
    crate::fsutil::reject_symlink_components(&stage)
        .map_err(|e| format!("unsafe source stage: {e}"))?;
    std::fs::create_dir_all(&stage).map_err(|e| format!("cannot stage inputs: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&stage, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let _source_stage = crate::source_io::StageGuard(stage.clone());
    let staged = crate::source_io::stage(cfg, &inputs, &stage, spec.max_records)
        .map_err(|e| format!("cannot prepare sources: {e}"))?;

    let opts = ExportOpts {
        inputs: staged,
        out_dir: out_dir.to_path_buf(),
        split_target: false,
        // `--history` is optional: when the run-history file has not been
        // created yet the export simply carries no policy hashes, exactly as
        // the CLI behaves when the flag is omitted.
        history: if cfg.schedule.history_path.exists() {
            Some(cfg.schedule.history_path.clone())
        } else {
            None
        },
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: spec.formats.clone(),
        dataset: spec.dataset.clone(),
        pdf_sample_rows: 12,
    };
    let report = crate::export::run_bounded(&opts, spec.max_export_bytes).map_err(|e| {
        let cause = format!("{e:#}")
            .replace(&out_dir.to_string_lossy().to_string(), "[delivery]")
            .replace(&stage.to_string_lossy().to_string(), "[source-stage]");
        if cause.contains("byte budget exceeded") {
            format!(
                "delivery is over the {} byte cap (write rejected before publication): {cause}",
                spec.max_export_bytes
            )
        } else {
            format!("export failed: {cause}")
        }
    })?;
    // The private source-stage guard cleans on success, error and cancellation.

    // Everything the delivery directory now holds is a candidate artifact;
    // the manifest tells us which ones it vouches for.
    let directory =
        std::fs::read_dir(out_dir).map_err(|e| format!("cannot list the delivery dir: {e}"))?;
    let mut names = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|e| format!("cannot read a delivery directory entry: {e}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with('.') {
            names.push(name);
        }
    }
    names.sort();

    let mut produced: Vec<ProducedFile> = Vec::new();
    let mut total = 0u64;
    for name in names {
        let path = out_dir.join(&name);
        let (sha, bytes) = hash_output_file(&path, spec.max_export_bytes.min(256 * 1024 * 1024))
            .map_err(|e| format!("cannot safely hash produced file {name}: {e}"))?;
        let role = if name == "manifest.json" || name == "manifest.json.sig" {
            ArtifactRole::Manifest
        } else if name == "checksums.txt" {
            ArtifactRole::Checksums
        } else if name.ends_with(".zip") {
            ArtifactRole::Bundle
        } else {
            ArtifactRole::Payload
        };
        total = total.saturating_add(bytes);
        if total > spec.max_export_bytes {
            return Err(format!(
                "the delivery is over the {} byte cap for this job",
                spec.max_export_bytes
            ));
        }
        produced.push(ProducedFile {
            path,
            name,
            role,
            bytes,
            sha256: sha,
        });
    }

    // The byte cap is checked against what is actually on disk. An over-cap
    // delivery is a failed job, not a truncated surprise.
    if total > spec.max_export_bytes {
        return Err(format!(
            "the delivery is {total} bytes, over the {} byte cap for this job",
            spec.max_export_bytes
        ));
    }

    let verification = if spec.verify {
        verify_dir(out_dir)
    } else {
        VerificationState::NotRequested
    };
    if let VerificationState::Failed { reason } = &verification {
        return Err(format!("verification failed: {reason}"));
    }

    let delivery_name = out_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "delivery".to_string());

    let result = ExportResult {
        records: report.manifest.records,
        unique_urls: report.manifest.unique_urls,
        formats: report
            .formats
            .iter()
            .map(|f| f.as_str().to_string())
            .collect(),
        files: produced
            .iter()
            .map(|f| ProducedFileName {
                name: f.name.clone(),
                role: f.role.as_str().to_string(),
                bytes: f.bytes,
                sha256: f.sha256.clone(),
            })
            .collect(),
        total_bytes: total,
        verification,
        delivery: delivery_name,
    };
    Ok((result, produced))
}

/// `harvest verify` semantics, without a signing key (the control plane never
/// holds one). Returns a [`VerificationState`] an artifact can carry.
pub fn verify_dir(dir: &Path) -> VerificationState {
    match crate::export::verify(dir, None) {
        Ok(v) => {
            if v.ok {
                VerificationState::Ok {
                    records: v.records,
                    records_basis: v.records_basis.to_string(),
                    files: v.files.len(),
                }
            } else {
                let reason = match &v.error {
                    Some(e) => e.clone(),
                    None => {
                        let bad: Vec<String> = v
                            .files
                            .iter()
                            .filter(|f| !(f.present && f.bytes_ok && f.sha_ok && f.lines_ok))
                            .map(|f| f.name.clone())
                            .collect();
                        if bad.is_empty() {
                            "the manifest's record total does not match the delivery".to_string()
                        } else {
                            format!("{} file(s) failed their manifest check", bad.len())
                        }
                    }
                };
                VerificationState::Failed { reason }
            }
        }
        Err(e) => VerificationState::Failed {
            reason: format!("verify could not run: {e}"),
        },
    }
}

#[cfg(test)]
mod tests;
