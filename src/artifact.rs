//! Artifact model for export deliveries — typed views over the manifest
//! `harvest export` writes and the outcome `harvest verify` computes, plus
//! the one classification rule shared by CLI, tests and any future report:
//! Verified / Tampered / Partial / MissingManifest / Unsigned+checked states.
//!
//! This module NEVER writes files or re-implements export/verify logic — the
//! implementations stay in [`crate::export`]. It exists so "what is a
//! delivery artifact" is stated once: identity (runs, targets, policy sha),
//! per-file promises (name, bytes, sha256, lines) and verification state.
//! Manifest wire fields are mirrored read-only from `export::Manifest`'s
//! serde output; a drift guard test
//! (`tests/artifact_integrity.rs`) exports with the real engine and parses
//! with this module, so the mirror can never silently diverge.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

/// One promised file in a delivery folder.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ArtifactFile {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
    pub lines: u64,
}

/// The delivery's identity — what it is about.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactIdentity {
    pub schema_version: u8,
    pub generated_ts: i64,
    pub runs: Vec<String>,
    pub records: u64,
    pub unique_urls: u64,
    pub skipped_lines: u64,
    /// latest `config_sha256` per run when the export embedded history
    pub policy_sha256: Option<std::collections::BTreeMap<String, String>>,
}

/// A parsed manifest (bounded: only the documented top-level fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    pub identity: ArtifactIdentity,
    pub files: Vec<ArtifactFile>,
    pub source_files: Vec<String>,
    pub split_target: bool,
    pub columns: Vec<String>,
}

/// Verification outcome classification. Strings in `signature` come from
/// `export::Verification`; we pattern-match the documented vocabulary, so
/// no second notion of "what valid means" is created here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationState {
    /// every manifest promise holds; signature not checked (no key given)
    VerifiedUnsignedUnchecked,
    /// every manifest promise holds AND the detached signature verified
    VerifiedSigned,
    /// every manifest promise holds BUT the folder was never signed
    VerifiedUnsigned,
    /// a manifest promise failed (checksum, bytes, line/record counts)
    Tampered,
    /// a promised file is absent, or records total mismatches
    Partial,
    /// manifest missing/unparseable — nothing else is trustworthy
    MissingManifest,
    /// signature present but failed to verify (wrong key or tampered bytes)
    SignatureInvalid,
}

impl ValidationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VerifiedUnsignedUnchecked => "verified:unsigned-unchecked",
            Self::VerifiedSigned => "verified:signed",
            Self::VerifiedUnsigned => "verified:unsigned-folder",
            Self::Tampered => "tampered",
            Self::Partial => "partial",
            Self::MissingManifest => "missing-manifest",
            Self::SignatureInvalid => "signature-invalid",
        }
    }
    /// Exit-code parity with `harvest verify`: only the three *Verified states
    /// are success. Anything else the operator must look at.
    pub fn is_ok(self) -> bool {
        matches!(
            self,
            Self::VerifiedSigned | Self::VerifiedUnsigned | Self::VerifiedUnsignedUnchecked
        )
    }
}

#[derive(Debug)]
pub enum ArtifactError {
    NotFound(String),
    Unreadable(String),
    Malformed(String),
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(p) => write!(f, "manifest.json not found in {p}"),
            Self::Unreadable(m) => write!(f, "manifest.json unreadable: {m}"),
            Self::Malformed(m) => write!(f, "manifest.json is not valid JSON: {m}"),
        }
    }
}

impl std::error::Error for ArtifactError {}

/// Parse a delivery's `manifest.json` into the typed view. Read-only.
pub fn load(dir: &Path) -> Result<Artifact, ArtifactError> {
    let path = safe_manifest_path(dir)?;
    let raw =
        std::fs::read_to_string(&path).map_err(|e| ArtifactError::Unreadable(e.to_string()))?;
    let v: Value =
        serde_json::from_str(&raw).map_err(|e| ArtifactError::Malformed(e.to_string()))?;
    from_value(&v).ok_or_else(|| ArtifactError::Malformed("shape mismatch".into()))
}

/// Validate the caller's folder path BEFORE touching the filesystem: the
/// manifest must be a direct child named exactly `manifest.json` (no
/// traversal, no symlinked names through `..`).
pub fn safe_manifest_path(dir: &Path) -> Result<std::path::PathBuf, ArtifactError> {
    let name_ok = dir.file_name().is_some();
    let joined = dir.join("manifest.json");
    let clean = joined.components().count() > 1
        && !joined
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    if !name_ok || !clean {
        return Err(ArtifactError::Unreadable(
            "refusing path with `..` components".into(),
        ));
    }
    Ok(joined)
}

pub fn from_value(v: &Value) -> Option<Artifact> {
    let files: Vec<ArtifactFile> = serde_json::from_value(v.get("files")?.clone()).ok()?;
    let identity = ArtifactIdentity {
        schema_version: v["v"].as_u64()? as u8,
        generated_ts: v["generated_ts"].as_i64()?,
        runs: serde_json::from_value(v["runs"].clone()).unwrap_or_default(),
        records: v["records"].as_u64()?,
        unique_urls: v["unique_urls"].as_u64()?,
        skipped_lines: v["skipped_lines"].as_u64()?,
        policy_sha256: v
            .get("policy_sha256")
            .and_then(|p| serde_json::from_value(p.clone()).ok()),
    };
    Some(Artifact {
        identity,
        files,
        source_files: v
            .get("source_files")
            .and_then(|s| serde_json::from_value(s.clone()).ok())
            .unwrap_or_default(),
        split_target: v["split_target"].as_bool()?,
        columns: v
            .get("columns")
            .and_then(|s| serde_json::from_value(s.clone()).ok())
            .unwrap_or_default(),
    })
}

/// Classify the outcome of `harvest verify` against the manifest state.
pub fn classify(verification: &crate::export::Verification) -> ValidationState {
    if verification.error.is_some() {
        return ValidationState::MissingManifest;
    }
    // any promised file missing ⇒ partial (before claiming tampering)
    if verification.files.iter().any(|f| !f.present) || !verification.records_ok {
        return ValidationState::Partial;
    }
    if !verification.ok {
        // A signature failure is NOT payload tampering: files can be intact
        // while the detached sig does not match (wrong key / re-signed).
        let payload_intact = verification
            .files
            .iter()
            .all(|f| f.present && f.bytes_ok && f.sha_ok && f.lines_ok)
            && verification.records_ok;
        if payload_intact && verification.signature.starts_with("invalid") {
            return ValidationState::SignatureInvalid;
        }
        // present but a promise failed: sha/bytes/line counts
        return ValidationState::Tampered;
    }
    match verification.signature.as_str() {
        "valid" => ValidationState::VerifiedSigned,
        "unsigned" => ValidationState::VerifiedUnsigned,
        s if s.starts_with("invalid") => ValidationState::SignatureInvalid,
        // "not checked (no --key given)" and any future wording: bytes are
        // proven against the manifest, but the signature was never examined.
        _ => ValidationState::VerifiedUnsignedUnchecked,
    }
}

/// Stable JSON shape for reports (audit of the audit, so to speak).
pub fn report_json(a: &Artifact, state: ValidationState) -> Value {
    serde_json::json!({
        "identity": {
            "v": a.identity.schema_version,
            "generated_ts": a.identity.generated_ts,
            "runs": a.identity.runs,
            "records": a.identity.records,
            "unique_urls": a.identity.unique_urls,
            "policy_sha256": a.identity.policy_sha256,
        },
        "files": a.files.iter().map(|f| serde_json::json!({
            "name": f.name, "sha256": f.sha256, "bytes": f.bytes, "lines": f.lines,
        })).collect::<Vec<_>>(),
        "state": state.as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        serde_json::json!({
            "v": 1,
            "generated_ts": 1700000000,
            "source_files": ["out/a.jsonl"],
            "split_target": false,
            "records": 12,
            "unique_urls": 9,
            "skipped_lines": 0,
            "runs": ["basic"],
            "targets": {"shop": 12},
            "policy_sha256": {"basic": "ab".repeat(32)},
            "columns": ["price", "title"],
            "files": [
                {"name": "records.jsonl", "sha256": "c".repeat(64), "bytes": 2048, "lines": 12},
                {"name": "records.csv", "sha256": "d".repeat(64), "bytes": 512, "lines": 13}
            ]
        })
    }

    #[test]
    fn typed_view_reads_every_manifest_promise() {
        let a = from_value(&sample()).expect("parse");
        assert_eq!(a.identity.records, 12);
        assert_eq!(a.files.len(), 2);
        assert_eq!(a.files[0].name, "records.jsonl");
        assert_eq!(a.identity.policy_sha256.unwrap()["basic"], "ab".repeat(32));
    }

    #[test]
    fn shape_drift_fails_closed() {
        let mut v = sample();
        v["files"] = Value::String("nope".into());
        assert!(from_value(&v).is_none(), "malformed files array is refused");
        v = sample();
        v["records"] = Value::Null;
        assert!(from_value(&v).is_none());
    }

    #[test]
    fn traversal_is_refused_before_fs() {
        let p = Path::new("/tmp/whatever/../..");
        assert!(safe_manifest_path(p).is_err());
        assert!(safe_manifest_path(Path::new("/tmp/deliver"))
            .unwrap()
            .ends_with("manifest.json"));
    }

    fn ver(
        ok: bool,
        files: Vec<crate::export::FileCheck>,
        sig: &str,
        err: Option<&str>,
    ) -> crate::export::Verification {
        crate::export::Verification {
            ok,
            records: 12,
            records_ok: true,
            records_basis: "jsonl-lines",
            files,
            extras: vec![],
            signature: sig.into(),
            error: err.map(|e| e.into()),
        }
    }

    #[test]
    fn classification_matches_verify_vocabulary() {
        use crate::export::FileCheck;
        let good = FileCheck {
            name: "x".into(),
            present: true,
            bytes_ok: true,
            sha_ok: true,
            lines_ok: true,
        };
        assert_eq!(
            classify(&ver(true, vec![good.clone()], "valid", None)),
            ValidationState::VerifiedSigned
        );
        assert_eq!(
            classify(&ver(true, vec![good.clone()], "unsigned", None)),
            ValidationState::VerifiedUnsigned
        );
        assert_eq!(
            classify(&ver(
                true,
                vec![good.clone()],
                "not checked (no --key given)",
                None
            )),
            ValidationState::VerifiedUnsignedUnchecked
        );
        let mut missing = good.clone();
        missing.present = false;
        assert_eq!(
            classify(&ver(
                false,
                vec![missing],
                "not checked (no --key given)",
                None
            )),
            ValidationState::Partial
        );
        let mut sha_bad = good.clone();
        sha_bad.sha_ok = false;
        assert_eq!(
            classify(&ver(
                false,
                vec![sha_bad],
                "not checked (no --key given)",
                None
            )),
            ValidationState::Tampered
        );
        assert_eq!(
            classify(&ver(false, vec![], "-", Some("manifest.json: missing"))),
            ValidationState::MissingManifest
        );
        assert_eq!(
            classify(&ver(
                true,
                vec![good],
                "invalid: signature does not match",
                None
            )),
            ValidationState::SignatureInvalid
        );
        // exit-code parity rule
        assert!(ValidationState::VerifiedUnsignedUnchecked.is_ok());
        assert!(!ValidationState::SignatureInvalid.is_ok());
        assert!(!ValidationState::Tampered.is_ok());
    }
}
