//! Persisted tenancy store: tenants, keys, and their limits.
//!
//! The store is one JSON document under the configured data directory,
//! rewritten atomically (private temp file + `renameat` inside the parent
//! directory descriptor) through [`crate::fsutil`]. Every mutation that
//! matters is persisted before it is acknowledged, so a crash between "key
//! created" and "response sent" cannot leave a key that exists only in
//! memory — or, worse, only in a half-written file.
//!
//! Corruption is fail-closed: a file that does not parse as *this* schema
//! version is refused at open time rather than silently replaced with an
//! empty store, which would look like "no keys" to an operator and let a
//! caller with an old key keep working.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use super::key::{ApiKey, GeneratedKey, KeyStatus};
use super::limits::KeyLimits;
use super::scope::ScopeSet;
use super::tenant::{Tenant, TenantId, TenantStatus};

/// On-disk schema version. Bump when the shape changes and add a migration
/// rather than accepting an old file as "the same thing".
pub const TENANCY_SCHEMA_VERSION: u32 = 1;
/// Throttle for `last_used_at` updates (seconds). A key used twice in one
/// second is still one observation.
pub const LAST_USED_THROTTLE_SECS: i64 = 60;

/// Why a store operation failed. Messages never contain key material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    Io(String),
    Corrupt(String),
    Invalid(String),
    NotFound(String),
    Conflict(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Io(m) => write!(f, "io: {m}"),
            StoreError::Corrupt(m) => write!(f, "corrupt store: {m}"),
            StoreError::Invalid(m) => write!(f, "invalid: {m}"),
            StoreError::NotFound(m) => write!(f, "not found: {m}"),
            StoreError::Conflict(m) => write!(f, "conflict: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

type StoreResult<T> = Result<T, StoreError>;

/// `(mtime_nanos, len)` for a file, used to detect an out-of-band rewrite.
fn fingerprint_of(meta: &std::fs::Metadata) -> (u64, u64) {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (mtime, meta.len())
}

/// The whole persisted document.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    tenants: Vec<Tenant>,
    keys: Vec<ApiKey>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            version: TENANCY_SCHEMA_VERSION,
            tenants: Vec::new(),
            keys: Vec::new(),
        }
    }
}

/// The tenancy store. `Arc`-shared; all access is behind one mutex so a
/// concurrent rotate and a lookup cannot interleave into a torn read.
#[derive(Debug)]
pub struct TenancyStore {
    path: PathBuf,
    state: Mutex<Snapshot>,
    /// Fingerprint of the snapshot as last observed on disk, `(mtime_nanos,
    /// len)`. A stat-based reload makes writes made by another process — the
    /// `harvest keys` CLI — visible to a long-running `serve` without a
    /// restart, which the operator workflow requires.
    loaded: Mutex<(u64, u64)>,
}

impl TenancyStore {
    /// Open (or create) the store at `path`.
    ///
    /// The parent directory is created privately (0700) and no-follow, and
    /// the snapshot file is loaded with a hard size cap so a hostile or
    /// truncated file cannot be turned into an unbounded allocation.
    pub fn open(path: impl Into<PathBuf>) -> StoreResult<Arc<Self>> {
        let path: PathBuf = path.into();
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        crate::fsutil::prepare_private_directory(parent)
            .map_err(|e| StoreError::Io(format!("cannot create data directory: {e}")))?;

        let mut snapshot = Snapshot::default();
        if path.exists() {
            let bytes = crate::fsutil::read_bounded(&path, MAX_SNAPSHOT_BYTES)
                .map_err(|e| StoreError::Io(format!("cannot read {}: {e}", path.display())))?;
            let text = std::str::from_utf8(&bytes)
                .map_err(|_| StoreError::Corrupt("snapshot is not valid UTF-8".into()))?;
            if text.trim().is_empty() {
                return Err(StoreError::Corrupt(
                    "snapshot file is empty; refusing to start from an empty store".into(),
                ));
            }
            snapshot = serde_json::from_str::<Snapshot>(text).map_err(|e| {
                StoreError::Corrupt(format!(
                    "snapshot does not match schema v{TENANCY_SCHEMA_VERSION}: {e}"
                ))
            })?;
            if snapshot.version != TENANCY_SCHEMA_VERSION {
                return Err(StoreError::Corrupt(format!(
                    "snapshot is schema v{} but this build understands v{TENANCY_SCHEMA_VERSION}",
                    snapshot.version
                )));
            }
        }

        let store = Arc::new(Self {
            path,
            state: Mutex::new(snapshot),
            loaded: Mutex::new((0, 0)),
        });
        store.persist()?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn persist(&self) -> StoreResult<()> {
        let snapshot = self
            .state
            .lock()
            .map_err(|_| StoreError::Io("tenancy store lock poisoned".into()))?;
        let body = serde_json::to_vec_pretty(&*snapshot)
            .map_err(|e| StoreError::Io(format!("cannot serialize snapshot: {e}")))?;
        crate::fsutil::write_private_atomic(&self.path, &body)
            .map_err(|e| StoreError::Io(format!("cannot write {}: {e}", self.path.display())))?;
        // Record what we just wrote, so the next `refresh` does not re-read
        // our own file.
        if let Ok(meta) = std::fs::metadata(&self.path) {
            if let Ok(mut loaded) = self.loaded.lock() {
                *loaded = fingerprint_of(&meta);
            }
        }
        Ok(())
    }

    fn lock_loaded(&self) -> StoreResult<std::sync::MutexGuard<'_, (u64, u64)>> {
        self.loaded
            .lock()
            .map_err(|_| StoreError::Io("tenancy store fingerprint lock poisoned".into()))
    }

    /// Re-read the snapshot when another process has rewritten it.
    ///
    /// The store is authoritative on disk and every mutation is persisted
    /// before it is acknowledged, so a stat-based reload is enough to make an
    /// out-of-band `harvest keys` write visible to a running server. The
    /// re-read is bounded by [`MAX_SNAPSHOT_BYTES`] and uses the same
    /// no-follow, atomic-write discipline as [`Self::open`].
    ///
    /// Failing to reload fails closed: the caller must not keep authorizing
    /// against a snapshot that is behind a revocation made elsewhere.
    pub fn refresh(&self) -> StoreResult<()> {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            // No file on disk: keep the in-memory snapshot rather than
            // emptying the store, which would look like "no keys".
            return Ok(());
        };
        let fp = fingerprint_of(&meta);
        let mut loaded = self.lock_loaded()?;
        if *loaded == fp {
            return Ok(());
        }
        let bytes = crate::fsutil::read_bounded(&self.path, MAX_SNAPSHOT_BYTES)
            .map_err(|e| StoreError::Io(format!("cannot read {}: {e}", self.path.display())))?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| StoreError::Corrupt("snapshot is not valid UTF-8".into()))?;
        let snapshot: Snapshot = serde_json::from_str(text).map_err(|e| {
            StoreError::Corrupt(format!(
                "snapshot does not match schema v{TENANCY_SCHEMA_VERSION}: {e}"
            ))
        })?;
        if snapshot.version != TENANCY_SCHEMA_VERSION {
            return Err(StoreError::Corrupt(format!(
                "snapshot is schema v{} but this build understands v{TENANCY_SCHEMA_VERSION}",
                snapshot.version
            )));
        }
        let mut state = self.lock()?;
        *state = snapshot;
        *loaded = fp;
        Ok(())
    }

    fn lock(&self) -> StoreResult<std::sync::MutexGuard<'_, Snapshot>> {
        self.state
            .lock()
            .map_err(|_| StoreError::Io("tenancy store lock poisoned".into()))
    }

    // ------------------------------------------------------------ tenants

    /// Create a tenant. Refuses a duplicate id (an operator may want a
    /// specific id; silently merging two tenants would merge their keys).
    pub fn create_tenant(&self, id: TenantId, name: &str) -> StoreResult<Tenant> {
        let tenant = Tenant::with_id(id, name).map_err(StoreError::Invalid)?;
        let mut s = self.lock()?;
        if s.tenants.iter().any(|t| t.id == tenant.id) {
            return Err(StoreError::Conflict(format!(
                "tenant `{}` already exists",
                tenant.id
            )));
        }
        s.tenants.push(tenant.clone());
        drop(s);
        self.persist()?;
        Ok(tenant)
    }

    /// Create a tenant with a generated id on an open store.
    pub fn new_tenant(&self, name: &str) -> StoreResult<Tenant> {
        let tenant = Tenant::create(name).map_err(StoreError::Invalid)?;
        self.create_tenant(tenant.id.clone(), name)
    }

    pub fn get_tenant(&self, id: &TenantId) -> Option<Tenant> {
        let _ = self.refresh();
        self.lock()
            .ok()?
            .tenants
            .iter()
            .find(|t| &t.id == id)
            .cloned()
    }

    pub fn list_tenants(&self) -> Vec<Tenant> {
        let _ = self.refresh();
        let mut tenants = self.lock().map(|s| s.tenants.clone()).unwrap_or_default();
        tenants.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        tenants
    }

    pub fn set_tenant_status(&self, id: &TenantId, status: TenantStatus) -> StoreResult<Tenant> {
        let mut s = self.lock()?;
        let tenant = s
            .tenants
            .iter_mut()
            .find(|t| &t.id == id)
            .ok_or_else(|| StoreError::NotFound(format!("tenant `{id}`")))?;
        tenant.status = status;
        let out = tenant.clone();
        drop(s);
        self.persist()?;
        Ok(out)
    }

    // ------------------------------------------------------------- keys

    /// Create a key for a tenant. The plaintext is returned once; only the
    /// hash is stored.
    pub fn create_key(
        &self,
        tenant_id: &str,
        scopes: ScopeSet,
        limits: KeyLimits,
        expires_at: Option<i64>,
    ) -> StoreResult<GeneratedKey> {
        let generated =
            ApiKey::create(tenant_id, scopes, limits, expires_at).map_err(StoreError::Invalid)?;
        let mut s = self.lock()?;
        if !s.tenants.iter().any(|t| t.id.as_str() == tenant_id) {
            return Err(StoreError::NotFound(format!("tenant `{tenant_id}`")));
        }
        if s.keys.iter().any(|k| k.id == generated.key.id) {
            return Err(StoreError::Conflict(format!(
                "key id `{}` is already taken",
                generated.key.id
            )));
        }
        s.keys.push(generated.key.clone());
        drop(s);
        self.persist()?;
        Ok(generated)
    }

    pub fn list_keys(&self, tenant_id: Option<&str>) -> Vec<ApiKey> {
        let _ = self.refresh();
        let mut keys = self
            .lock()
            .map(|s| {
                s.keys
                    .iter()
                    .filter(|k| tenant_id.is_none_or(|t| k.tenant_id == t))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        keys.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        keys
    }

    pub fn get_key(&self, id: &str) -> Option<ApiKey> {
        let _ = self.refresh();
        let parsed = super::key::KeyId::parse(id).ok()?;
        self.lock()
            .ok()?
            .keys
            .iter()
            .find(|k| k.id == parsed)
            .cloned()
    }

    /// Rotate a key: the old record is revoked and a fresh one is minted
    /// with the same tenant, scopes and limits.
    pub fn rotate_key(&self, id: &str) -> StoreResult<GeneratedKey> {
        let parsed = super::key::KeyId::parse(id).map_err(StoreError::Invalid)?;
        let mut s = self.lock()?;
        let Some(index) = s.keys.iter().position(|k| k.id == parsed) else {
            return Err(StoreError::NotFound(format!("key `{id}`")));
        };
        let existing = s.keys[index].clone();
        let generated =
            ApiKey::rotate(&existing, existing.expires_at).map_err(StoreError::Invalid)?;
        s.keys[index].status = KeyStatus::Revoked;
        s.keys[index].last_used_at = None;
        s.keys.push(generated.key.clone());
        drop(s);
        self.persist()?;
        Ok(generated)
    }

    pub fn revoke_key(&self, id: &str) -> StoreResult<ApiKey> {
        self.set_key_status(id, KeyStatus::Revoked)
    }

    pub fn suspend_key(&self, id: &str) -> StoreResult<ApiKey> {
        self.set_key_status(id, KeyStatus::Suspended)
    }

    pub fn set_key_status(&self, id: &str, status: KeyStatus) -> StoreResult<ApiKey> {
        let parsed = super::key::KeyId::parse(id).map_err(StoreError::Invalid)?;
        let mut s = self.lock()?;
        let key = s
            .keys
            .iter_mut()
            .find(|k| k.id == parsed)
            .ok_or_else(|| StoreError::NotFound(format!("key `{id}`")))?;
        key.status = status;
        let out = key.clone();
        drop(s);
        self.persist()?;
        Ok(out)
    }

    /// Update the limits of an existing key. Limits are validated first:
    /// an operator cannot push a key past the bounds the rate limiter
    /// assumes.
    pub fn set_limits(&self, id: &str, limits: KeyLimits) -> StoreResult<ApiKey> {
        limits.validate().map_err(StoreError::Invalid)?;
        let parsed = super::key::KeyId::parse(id).map_err(StoreError::Invalid)?;
        let mut s = self.lock()?;
        let key = s
            .keys
            .iter_mut()
            .find(|k| k.id == parsed)
            .ok_or_else(|| StoreError::NotFound(format!("key `{id}`")))?;
        key.limits = limits;
        let out = key.clone();
        drop(s);
        self.persist()?;
        Ok(out)
    }

    /// Look up a key by the plaintext it presents. Every candidate digest is
    /// compared in constant time; nothing about which key matched (or how
    /// far along a guess got) is observable from the outside.
    pub fn lookup_by_key(&self, presented: &str) -> Option<ApiKey> {
        let _ = self.refresh();
        if presented.is_empty() || presented.len() > 512 {
            return None;
        }
        let keys = self.lock().ok()?.keys.clone();
        for key in &keys {
            if super::key::verify_key(&key.key_hash, presented) {
                return Some(key.clone());
            }
        }
        None
    }

    /// Record a successful use. Throttled: the stored `last_used_at` only
    /// moves when at least `LAST_USED_THROTTLE_SECS` elapsed, so a hot key
    /// does not rewrite the store on every request.
    pub fn touch_last_used(&self, id: &str) -> StoreResult<()> {
        let parsed = super::key::KeyId::parse(id).map_err(StoreError::Invalid)?;
        let now = crate::ids::Timestamp::now().as_secs();
        let mut s = self.lock()?;
        let Some(key) = s.keys.iter_mut().find(|k| k.id == parsed) else {
            return Ok(());
        };
        let updated = match key.last_used_at {
            Some(prev) => now - prev >= LAST_USED_THROTTLE_SECS,
            None => true,
        };
        if updated {
            key.last_used_at = Some(now);
            drop(s);
            self.persist()?;
        }
        Ok(())
    }

    /// Key + tenant counts for `/v1/admin/keys` and diagnostics.
    pub fn stats(&self) -> (usize, usize) {
        let s = self.lock();
        match s {
            Ok(s) => (s.tenants.len(), s.keys.len()),
            Err(_) => (0, 0),
        }
    }
}

/// Snapshot size cap. A tenancy document with thousands of keys is a few
/// hundred KiB; anything larger is not a tenancy document.
pub const MAX_SNAPSHOT_BYTES: u64 = 64 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::scope::Scope;

    fn tmp(name: &str) -> PathBuf {
        let t = std::env::temp_dir().join(format!("harvest-tenancy-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        t
    }

    #[test]
    fn round_trip_persists_and_reloads() {
        let dir = tmp("roundtrip");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::from(vec![Scope::Scrape, Scope::UsageRead]),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        assert!(store.get_key(g.key.id.as_str()).is_some());

        // Reopen: the store must read back what was written.
        let store2 = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        assert_eq!(store2.stats(), (1, 1));
        let back = store2.get_key(g.key.id.as_str()).unwrap();
        assert_eq!(back.key_hash, g.key.key_hash);
        assert_eq!(back.prefix, g.key.prefix);
        assert_eq!(back.scopes, g.key.scopes);
        assert_ne!(back.key_hash, g.plaintext);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lookup_by_presented_key_finds_the_right_record() {
        let dir = tmp("lookup");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        assert!(store
            .lookup_by_key(&g.plaintext)
            .is_some_and(|k| k.id == g.key.id));
        assert!(store.lookup_by_key("hv_live_0000").is_none());
        assert!(store.lookup_by_key("").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rotate_revokes_the_old_key_and_keeps_identity() {
        let dir = tmp("rotate");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        let rotated = store.rotate_key(g.key.id.as_str()).unwrap();

        let old = store.get_key(g.key.id.as_str()).unwrap();
        assert_eq!(old.status, KeyStatus::Revoked);
        assert!(!old.is_usable(crate::ids::Timestamp::now().as_secs()));
        assert!(store
            .lookup_by_key(&g.plaintext)
            .is_none_or(|k| k.status != KeyStatus::Active));

        let fresh = store.get_key(rotated.key.id.as_str()).unwrap();
        assert_eq!(fresh.status, KeyStatus::Active);
        assert_eq!(fresh.tenant_id, tenant.id.as_str());
        assert!(fresh.is_usable(crate::ids::Timestamp::now().as_secs()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn revoke_and_suspend_change_usability() {
        let dir = tmp("status");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        let now = crate::ids::Timestamp::now().as_secs();
        assert!(store.get_key(g.key.id.as_str()).unwrap().is_usable(now));

        let suspended = store.suspend_key(g.key.id.as_str()).unwrap();
        assert_eq!(suspended.status, KeyStatus::Suspended);
        assert!(!suspended.is_usable(now));

        let revoked = store.revoke_key(g.key.id.as_str()).unwrap();
        assert_eq!(revoked.status, KeyStatus::Revoked);
        assert!(!revoked.is_usable(now));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tenant_status_gates_usability_at_the_auth_layer() {
        let dir = tmp("tenant-status");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        store
            .set_tenant_status(&tenant.id, TenantStatus::Suspended)
            .unwrap();
        assert_eq!(
            store.get_tenant(&tenant.id).unwrap().status,
            TenantStatus::Suspended
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_snapshot_fails_closed() {
        let dir = tmp("corrupt");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tenancy.json"), b"{ not json").unwrap();
        let err = TenancyStore::open(dir.join("tenancy.json")).unwrap_err();
        assert!(matches!(err, StoreError::Corrupt(_)), "got {err:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_snapshot_fails_closed() {
        let dir = tmp("empty");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tenancy.json"), b"   \n").unwrap();
        assert!(matches!(
            TenancyStore::open(dir.join("tenancy.json")).unwrap_err(),
            StoreError::Corrupt(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_schema_version_fails_closed() {
        let dir = tmp("version");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tenancy.json"),
            b"{\"version\":999,\"tenants\":[],\"keys\":[]}",
        )
        .unwrap();
        assert!(matches!(
            TenancyStore::open(dir.join("tenancy.json")).unwrap_err(),
            StoreError::Corrupt(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_fields_in_snapshot_fail_closed() {
        let dir = tmp("unknown");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("tenancy.json"),
            b"{\"version\":1,\"tenants\":[],\"keys\":[],\"surprise\":1}",
        )
        .unwrap();
        assert!(matches!(
            TenancyStore::open(dir.join("tenancy.json")).unwrap_err(),
            StoreError::Corrupt(_)
        ));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn limits_edits_are_validated() {
        let dir = tmp("limits");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        let bad = KeyLimits {
            rps: -1.0,
            ..KeyLimits::default()
        };
        assert!(store.set_limits(g.key.id.as_str(), bad).is_err());

        let good = KeyLimits {
            rps: 20.0,
            allowed_domains: vec!["example.com".into()],
            ..KeyLimits::default()
        };
        let updated = store.set_limits(g.key.id.as_str(), good).unwrap();
        assert_eq!(updated.limits.rps, 20.0);
        assert!(updated.limits.allows_domain("www.example.com"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn duplicate_tenant_ids_are_refused() {
        let dir = tmp("dupe");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let id = TenantId::parse("acme").unwrap();
        store.create_tenant(id.clone(), "Acme").unwrap();
        assert!(store.create_tenant(id, "Acme Again").is_err());
        assert_eq!(store.list_tenants().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keys_for_missing_tenant_are_refused() {
        let dir = tmp("notenant");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        assert!(store
            .create_key("t-nope", ScopeSet::default(), KeyLimits::default(), None)
            .is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn last_used_is_throttled() {
        let dir = tmp("throttle");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::default(),
                KeyLimits::default(),
                None,
            )
            .unwrap();
        store.touch_last_used(g.key.id.as_str()).unwrap();
        let first = store.get_key(g.key.id.as_str()).unwrap();
        assert!(first.last_used_at.is_some());
        // A second touch inside the throttle window must not rewrite state.
        let before = std::fs::read(dir.join("tenancy.json")).unwrap();
        store.touch_last_used(g.key.id.as_str()).unwrap();
        let after = std::fs::read(dir.join("tenancy.json")).unwrap();
        assert_eq!(before, after);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
