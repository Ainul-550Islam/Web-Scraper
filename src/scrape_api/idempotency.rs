//! Idempotency keys.
//!
//! A caller may send `idempotency-key` (header) or `idempotency_key` (body).
//! Within the retention window, a repeat of the same key returns the stored
//! response instead of doing the work twice — which matters because the work
//! is billed.
//!
//! The store is a single append-only JSONL file under a mutex: the check and
//! the store are one critical section, so two concurrent requests with the
//! same key cannot both miss. Records are keyed by the SHA-256 of the key so
//! the file never contains a caller's secret, and bounded by
//! `idempotency_max_records` so it cannot grow without limit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::config::ManagedApiCfg;
use crate::fsutil;

/// One stored response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// SHA-256 of the caller's key, never the key itself.
    pub key_hash: String,
    /// `created_at_ms` + `retention_ms`, used to drop expired records.
    pub created_at_ms: u64,
    pub expires_at_ms: u64,
    /// The exact response body that was returned the first time.
    pub body: String,
    pub status: u16,
}

/// Outcome of a `put_if_absent`: either a replay of an existing record or a
/// brand-new one.
#[derive(Debug, Clone)]
pub enum Insert {
    Inserted(Record),
    Replayed(Record),
}

impl Insert {
    pub fn record(&self) -> &Record {
        match self {
            Self::Inserted(r) | Self::Replayed(r) => r,
        }
    }

    pub fn replayed(&self) -> bool {
        matches!(self, Self::Replayed(_))
    }
}

/// The on-disk shape.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    #[serde(default)]
    records: Vec<Record>,
}

pub struct IdempotencyStore {
    dir: PathBuf,
    path: PathBuf,
    retention_ms: u64,
    max_records: u32,
    lock: Mutex<()>,
    cache: Mutex<Option<BTreeMap<String, Record>>>,
}

impl IdempotencyStore {
    pub fn open(cfg: &ManagedApiCfg, dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref();
        fsutil::prepare_private_directory(dir)
            .with_context(|| format!("cannot prepare idempotency directory {}", dir.display()))?;
        let path = dir.join("idempotency.jsonl");
        let store = Self {
            dir: dir.to_path_buf(),
            path,
            retention_ms: cfg.idempotency_retention_ms,
            max_records: cfg.idempotency_max_records,
            lock: Mutex::new(()),
            cache: Mutex::new(None),
        };
        // Load lazily on first use so an operator can point this at a live
        // directory without paying for it at startup.
        store.load()?;
        Ok(store)
    }

    fn load(&self) -> Result<()> {
        let mut map: BTreeMap<String, Record> = BTreeMap::new();
        if self.path.exists() {
            let text = fsutil::read_bounded(&self.path, 4 * 1024 * 1024)
                .with_context(|| format!("cannot read idempotency file {}", self.path.display()))?;
            for line in String::from_utf8_lossy(&text).lines() {
                if line.trim().is_empty() {
                    continue;
                }
                let rec: Record = serde_json::from_str(line)?;
                map.insert(rec.key_hash.clone(), rec);
            }
        }
        let mut guard = self.cache();
        *guard = Some(map);
        Ok(())
    }

    fn cache(&self) -> std::sync::MutexGuard<'_, Option<BTreeMap<String, Record>>> {
        let mut guard = self.cache.lock().unwrap();
        if guard.is_none() {
            // The lock is held; a nested call is a bug, but `load` only reads
            // the file and does not re-enter the cache lock.
            *guard = Some(BTreeMap::new());
        }
        guard
    }

    /// Read a stored response without creating one.
    pub fn lookup(&self, key: &str) -> Result<Option<Record>> {
        let _guard = self.lock.lock().unwrap();
        let mut cache = self.cache();
        let map = cache.as_mut().unwrap();
        map.retain(|_, r| r.expires_at_ms > now_ms());
        Ok(map.get(&key_hash(key)).cloned())
    }

    /// Store `body` under `key`, or report that it was already there.
    pub fn store(&self, key: &str, status: u16, body: &str) -> Result<Insert> {
        self.put_if_absent(key, status, body)
    }

    /// Atomically store `body` under `key`, or return the existing record.
    pub fn put_if_absent(&self, key: &str, status: u16, body: &str) -> Result<Insert> {
        let _guard = self.lock.lock().unwrap();
        let key_hash = key_hash(key);
        let now = now_ms();

        let mut cache = self.cache();
        let cache_ref = cache.as_mut().unwrap();
        cache_ref.retain(|_, r| r.expires_at_ms > now);

        if let Some(existing) = cache_ref.get(&key_hash) {
            return Ok(Insert::Replayed(existing.clone()));
        }

        let record = Record {
            key_hash,
            created_at_ms: now,
            expires_at_ms: now.saturating_add(self.retention_ms),
            body: body.to_string(),
            status,
        };

        // Bound the file: drop the oldest records beyond the cap so a long
        // lived deployment cannot grow this without bound.
        if cache_ref.len() >= self.max_records as usize {
            let excess = cache_ref.len() + 1 - self.max_records as usize;
            let drop_keys: Vec<String> = cache_ref
                .iter()
                .take(excess)
                .map(|(k, _)| k.clone())
                .collect();
            for k in drop_keys {
                cache_ref.remove(&k);
            }
        }

        cache_ref.insert(record.key_hash.clone(), record.clone());
        self.persist(cache_ref)?;
        Ok(Insert::Inserted(record))
    }

    fn persist(&self, cache: &BTreeMap<String, Record>) -> Result<()> {
        fsutil::prepare_private_directory(&self.dir)?;
        let body = serde_json::to_string(&Snapshot {
            records: cache.values().cloned().collect(),
        })?;
        fsutil::write_private_atomic(&self.path, body.as_bytes())?;
        Ok(())
    }

    /// How many records are currently held.
    pub fn len(&self) -> usize {
        let guard = self.cache();
        guard.as_ref().map(|c| c.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn key_hash(key: &str) -> String {
    let mut h = Sha256::new();
    h.update(key.as_bytes());
    let digest = h.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
