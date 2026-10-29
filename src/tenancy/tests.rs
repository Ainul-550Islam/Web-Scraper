//! Cross-cutting tests for the tenancy subsystem.
//!
//! The per-unit tests live next to their units (`tenant.rs`, `key.rs`, …);
//! this file holds the invariants that only become visible when the pieces
//! are combined: the *persisted bytes* never contain a plaintext key, the
//! store survives a reopen, a corrupted document fails closed, and a
//! principal's scopes are enforced through the auth boundary rather than by
//! each handler remembering to check.

use std::path::PathBuf;

use crate::tenancy::auth::authenticate;
use crate::tenancy::key::{hash_key, verify_key, ApiKey};
use crate::tenancy::limits::KeyLimits;
use crate::tenancy::scope::{Scope, ScopeSet};
use crate::tenancy::store::{StoreError, TenancyStore};
use crate::tenancy::tenant::TenantStatus;

fn tmp(name: &str) -> PathBuf {
    let t = std::env::temp_dir().join(format!(
        "harvest-tenancy-tests-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&t);
    t
}

fn store(name: &str) -> (PathBuf, std::sync::Arc<TenancyStore>) {
    let dir = tmp(name);
    let s = TenancyStore::open(dir.join("tenancy.json")).expect("store opens");
    (dir, s)
}

fn key_for(store: &TenancyStore, scopes: ScopeSet) -> (crate::tenancy::tenant::Tenant, String) {
    let tenant = store.new_tenant("Acme").unwrap();
    let g = store
        .create_key(tenant.id.as_str(), scopes, KeyLimits::default(), None)
        .unwrap();
    (tenant, g.plaintext)
}

#[test]
fn persisted_bytes_never_contain_a_plaintext_key() {
    let (dir, store) = store("plaintext-absent");
    let (_, plaintext) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));

    let bytes = std::fs::read(store.path()).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        !text.contains(&plaintext),
        "the plaintext key must not appear anywhere in the store file"
    );
    assert!(text.contains(&hash_key(&plaintext)));
    assert!(text.contains("key_hash"));

    // Same guarantee at the record level: the serialized key carries a hash
    // and a display prefix, never the secret.
    let record = store.lookup_by_key(&plaintext).unwrap();
    let json = serde_json::to_string(&record).unwrap();
    assert!(!json.contains(&plaintext));
    assert!(json.contains(&record.key_hash));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn verification_is_constant_over_digest_length() {
    let g = ApiKey::create("t-1", ScopeSet::default(), KeyLimits::default(), None).unwrap();
    assert!(verify_key(&g.key.key_hash, &g.plaintext));
    assert!(!verify_key(
        &g.key.key_hash,
        &g.plaintext[..KEY_PREFIX_LEN - 1]
    ));
    assert!(!verify_key(&g.key.key_hash, &format!("{}x", g.plaintext)));
    assert!(!verify_key("deadbeef", &g.plaintext));
}

const KEY_PREFIX_LEN: usize = 12;

#[test]
fn rotate_revokes_and_preserves_tenant_identity() {
    let (dir, store) = store("rotate");
    let (tenant, plaintext) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));
    let old_id = store.lookup_by_key(&plaintext).unwrap().id;

    let rotated = store.rotate_key(old_id.as_str()).unwrap();
    let now = crate::ids::Timestamp::now().as_secs();

    let old = store.get_key(old_id.as_str()).unwrap();
    assert_eq!(old.status, crate::tenancy::key::KeyStatus::Revoked);
    assert!(!old.is_usable(now));

    let fresh = store.get_key(rotated.key.id.as_str()).unwrap();
    assert_eq!(fresh.tenant_id, tenant.id.as_str());
    assert_eq!(fresh.scopes, rotated.key.scopes);
    assert_eq!(fresh.limits, rotated.key.limits);
    assert!(fresh.is_usable(now));
    assert!(authenticate(
        store.as_ref(),
        Some(&format!("Bearer {}", rotated.plaintext)),
        None
    )
    .is_ok());
    assert!(authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn revoke_and_suspend_stop_a_key_immediately() {
    let (dir, store) = store("revoke-suspend");
    let (_, plaintext) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));
    let id = store.lookup_by_key(&plaintext).unwrap().id;

    assert!(authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).is_ok());
    store.suspend_key(id.as_str()).unwrap();
    let e = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
    assert_eq!(e.status, 403);

    store.revoke_key(id.as_str()).unwrap();
    let e = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
    assert_eq!(e.status, 403);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn expiry_is_enforced_through_authentication() {
    let (dir, store) = store("expiry");
    let tenant = store.new_tenant("Acme").unwrap();
    let now = crate::ids::Timestamp::now().as_secs();
    let g = store
        .create_key(
            tenant.id.as_str(),
            ScopeSet::from(vec![Scope::Scrape]),
            KeyLimits::default(),
            Some(now + 3600),
        )
        .unwrap();
    assert!(authenticate(
        store.as_ref(),
        Some(&format!("Bearer {}", g.plaintext)),
        None
    )
    .is_ok());

    let expired = store
        .create_key(
            tenant.id.as_str(),
            ScopeSet::from(vec![Scope::Scrape]),
            KeyLimits::default(),
            Some(now - 1),
        )
        .unwrap();
    let e = authenticate(
        store.as_ref(),
        Some(&format!("Bearer {}", expired.plaintext)),
        None,
    )
    .unwrap_err();
    assert_eq!(e.status, 403);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scope_checks_are_enforced_at_the_boundary() {
    let (dir, store) = store("scopes");
    let (_, plaintext) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));
    let p = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap();

    assert!(p.require(Scope::Scrape).is_ok());
    assert_eq!(p.require(Scope::UsageRead).unwrap_err().status, 403);
    assert_eq!(p.require(Scope::JobsRead).unwrap_err().status, 403);
    assert_eq!(p.require(Scope::JobsWrite).unwrap_err().status, 403);
    assert_eq!(p.require(Scope::KeysAdmin).unwrap_err().status, 403);
    assert!(p.require_all(&[Scope::Scrape, Scope::UsageRead]).is_err());
    assert!(p.require_all(&[Scope::Scrape]).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn persistence_round_trips_through_a_reopen() {
    let (dir, store) = store("roundtrip");
    let tenant = store.new_tenant("Acme").unwrap();
    let g = store
        .create_key(
            tenant.id.as_str(),
            ScopeSet::from(vec![Scope::Scrape, Scope::UsageRead]),
            KeyLimits::default(),
            None,
        )
        .unwrap();
    store
        .set_limits(
            g.key.id.as_str(),
            KeyLimits {
                rps: 42.0,
                ..KeyLimits::default()
            },
        )
        .unwrap();

    let reopened = TenancyStore::open(store.path()).unwrap();
    assert_eq!(reopened.stats(), store.stats());
    let back = reopened.get_key(g.key.id.as_str()).unwrap();
    assert_eq!(back, store.get_key(g.key.id.as_str()).unwrap());
    assert_eq!(back.limits.rps, 42.0);
    assert!(authenticate(
        reopened.as_ref(),
        Some(&format!("Bearer {}", g.plaintext)),
        None
    )
    .is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupted_files_fail_closed() {
    let cases: [(&str, &[u8]); 5] = [
        ("not-json", b"{ not json".as_slice()),
        ("empty", b"   ".as_slice()),
        (
            "wrong-version",
            b"{\"version\":42,\"tenants\":[],\"keys\":[]}".as_slice(),
        ),
        (
            "unknown-field",
            b"{\"version\":1,\"tenants\":[],\"keys\":[],\"x\":1}".as_slice(),
        ),
        (
            "truncated",
            b"{\"version\":1,\"tenants\":[{\"id\":\"a\"".as_slice(),
        ),
    ];
    for (name, body) in cases {
        let dir = tmp(&format!("corrupt-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tenancy.json"), body).unwrap();
        let err = TenancyStore::open(dir.join("tenancy.json")).unwrap_err();
        assert!(
            matches!(err, StoreError::Corrupt(_)),
            "{name}: expected Corrupt, got {err:?}"
        );
        // The bad file is left in place: an operator must be able to inspect
        // it. We never overwrite evidence with a blank store.
        assert_eq!(
            std::fs::read(dir.join("tenancy.json")).unwrap(),
            body.to_vec()
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn tenant_suspension_blocks_its_keys() {
    let (dir, store) = store("tenant-suspend");
    let (tenant, plaintext) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));
    assert!(authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).is_ok());
    store
        .set_tenant_status(&tenant.id, TenantStatus::Suspended)
        .unwrap();
    let e = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
    assert_eq!(e.status, 403);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn keys_are_isolated_between_tenants() {
    let (dir, store) = store("tenant-isolation");
    let (tenant_a, plaintext_a) = key_for(&store, ScopeSet::from(vec![Scope::Scrape]));
    let g_b = store
        .create_key(
            tenant_a.id.as_str(),
            ScopeSet::from(vec![Scope::JobsRead]),
            KeyLimits::default(),
            None,
        )
        .unwrap();

    let keys_a = store.list_keys(Some(tenant_a.id.as_str()));
    assert_eq!(keys_a.len(), 2);
    assert!(keys_a.iter().any(|k| k.id == g_b.key.id));

    let keys_other = store.list_keys(Some("t-does-not-exist"));
    assert!(keys_other.is_empty());

    // Both keys belong to the same tenant, so both principals resolve to it;
    // the store hands out exactly the keys of the caller's own tenant.
    let p_a = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext_a}")), None).unwrap();
    let p_b = authenticate(
        store.as_ref(),
        Some(&format!("Bearer {}", g_b.plaintext)),
        None,
    )
    .unwrap();
    assert_eq!(p_a.tenant_id(), tenant_a.id.as_str());
    assert_eq!(p_b.tenant_id(), tenant_a.id.as_str());
    assert_ne!(p_a.key_id(), p_b.key_id());
    std::fs::remove_dir_all(&dir).unwrap();
}
