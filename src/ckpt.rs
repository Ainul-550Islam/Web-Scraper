//! Checkpoint / resume for long crawls.
//!
//! A crawl of hours (or days) should survive the process dying — operator
//! ctrl-c, deploy, OOM, whatever. Every `ckpt_every_pages` records (or
//! `ckpt_every_secs`, whichever comes first) the engine snapshots its
//! discovery state — pending queue, per-target visited URLs, content
//! fingerprints, budget counters — into a single JSON file written
//! *atomically* (temp file + rename, so a kill mid-write can never leave a
//! half-file). `harvest run --resume` loads it and continues exactly where
//! the last process left off: already-enqueued URLs are never fetched again.
//!
//! Two deliberate contracts:
//! * **Policy attestation**: a checkpoint stores `policy_sha256`; resuming
//!   with a changed config is *refused*, not silently accepted — resuming a
//!   crawl under different politeness rules than it started with would
//!   invalidate the compliance story (and the audit history).
//! * **At-most-one-checkpoint of work lost** on hard kill: a page that was
//!   mid-flight when SIGKILL landed is already in `visited` (mark-on-enqueue)
//!   but maybe not yet written. Resume will not re-fetch it. The DLQ covers
//!   refusals; this window covers process death and is bounded by the
//!   checkpoint cadence you configure.
//!
//! On a *complete* run the checkpoint is deleted — "nothing left to resume".

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Checkpoint layout version. Bump on incompatible shape changes.
pub const VERSION: u32 = 1;
const MAX_CHECKPOINT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingJob {
    pub url: String,
    pub depth: usize,
    pub target: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TargetState {
    /// Target id — cross-checked against config on load (order + name).
    pub id: String,
    /// Dedup keys of every URL already enqueued (not just fetched).
    pub visited: Vec<String>,
    /// Content fingerprints already written for this target.
    pub fp: Vec<String>,
    /// Pages written so far (feeds the per-target `max_pages` budget).
    pub written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checkpoint {
    pub v: u32,
    pub run: String,
    pub policy_sha256: String,
    pub saved_ts: i64,
    /// Global pages written (feeds the global `max_pages` expansion cap).
    pub global_pages: usize,
    pub pending: Vec<PendingJob>,
    pub targets: Vec<TargetState>,
}

impl Checkpoint {
    pub fn new(run: &str, policy_sha256: &str) -> Self {
        Self {
            v: VERSION,
            run: run.to_string(),
            policy_sha256: policy_sha256.to_string(),
            saved_ts: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
            global_pages: 0,
            pending: Vec::new(),
            targets: Vec::new(),
        }
    }

    /// Atomic: write beside-file, then rename. A crash anywhere in here
    /// leaves either the previous checkpoint or the new one — never a mix.
    pub fn save(&self, path: &Path) -> Result<()> {
        let body = serde_json::to_vec(self).context("serialize checkpoint")?;
        if body.len() as u64 > MAX_CHECKPOINT_BYTES {
            bail!("checkpoint exceeds the {MAX_CHECKPOINT_BYTES}-byte limit");
        }
        crate::fsutil::write_private_atomic(path, &body)
            .with_context(|| format!("write checkpoint {}", path.display()))?;
        Ok(())
    }

    /// Load and cross-validate against the live config. Every failure mode
    /// is a hard error on purpose: guessing here means quietly re-crawling
    /// (or *not* crawling) pages the operator believes are handled.
    pub fn load_validated(path: &Path, cfg: &crate::Config) -> Result<Self> {
        let bytes = crate::fsutil::read_bounded(path, MAX_CHECKPOINT_BYTES)
            .with_context(|| format!("read checkpoint {}", path.display()))?;
        let raw = String::from_utf8(bytes).context("checkpoint is not UTF-8")?;
        let ck: Checkpoint = serde_json::from_str(&raw).with_context(|| {
            format!(
                "corrupt checkpoint {} (not valid JSON for v{VERSION})",
                path.display()
            )
        })?;
        if ck.v != VERSION {
            bail!(
                "checkpoint version {} not supported (engine writes v{VERSION})",
                ck.v
            );
        }
        if ck.run != cfg.name {
            bail!(
                "checkpoint belongs to run `{}`, this config is `{}` — refusing to merge",
                ck.run,
                cfg.name
            );
        }
        if ck.policy_sha256 != cfg.hash() {
            bail!(
                "config changed since the checkpoint was written (policy sha {} vs {}) — \
                 politeness rules a crawl started under are the rules it must finish under; \
                 start a fresh run instead of resuming",
                short(&ck.policy_sha256),
                short(&cfg.hash())
            );
        }
        if ck.targets.len() != cfg.targets.len() {
            bail!(
                "checkpoint has {} target(s), config has {}",
                ck.targets.len(),
                cfg.targets.len()
            );
        }
        for (st, tc) in ck.targets.iter().zip(cfg.targets.iter()) {
            if st.id != tc.id {
                bail!(
                    "checkpoint target order changed (`{}` vs `{}`)",
                    st.id,
                    tc.id
                );
            }
        }
        Ok(ck)
    }

    /// True if there is discovery work left (queue non-empty) after restore.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}

fn short(sha: &str) -> String {
    sha.chars().take(12).collect()
}

#[cfg(test)]
mod tests;
