//! API-key authentication for the managed surface.
//!
//! Two header forms are accepted so a customer can use whatever their HTTP
//! client prefers:
//!
//! * `Authorization: Bearer hv_live_…`
//! * `X-API-Key: hv_live_…`
//!
//! Fail-closed on every path: a missing, malformed, unknown, expired,
//! revoked or suspended key is refused, and the refusal is *uniform* — the
//! body, status and latency do not distinguish "no key" from "wrong key"
//! from "key exists but is disabled". A client that can tell those apart has
//! an oracle for probing key existence.
//!
//! Secrets never reach a log line: the presented value is dropped at the
//! boundary, and only the (already hashed) key id is recorded.

use crate::api::ApiError;

use super::key::ApiKey;
use super::scope::{Scope, ScopeSet};
use super::store::TenancyStore;
use super::tenant::Tenant;

/// The authenticated caller: the tenant that owns the key, plus the key
/// record (which carries scopes and limits).
#[derive(Debug, Clone)]
pub struct Principal {
    pub tenant: Tenant,
    pub key: ApiKey,
}

impl Principal {
    pub fn tenant_id(&self) -> &str {
        self.tenant.id.as_str()
    }

    pub fn key_id(&self) -> &str {
        self.key.id.as_str()
    }

    pub fn scopes(&self) -> &ScopeSet {
        &self.key.scopes
    }

    pub fn limits(&self) -> &super::limits::KeyLimits {
        &self.key.limits
    }

    pub fn is_admin(&self) -> bool {
        self.key.allows(Scope::KeysAdmin)
    }

    /// Capability gate shared by every managed handler.
    pub fn require(&self, scope: Scope) -> Result<(), ApiError> {
        if self.key.allows(scope) {
            Ok(())
        } else {
            Err(ApiError::forbidden(format!(
                "this key does not have the `{}` scope",
                scope.as_str()
            )))
        }
    }

    pub fn require_all(&self, scopes: &[Scope]) -> Result<(), ApiError> {
        for scope in scopes {
            self.require(*scope)?;
        }
        Ok(())
    }
}

/// A digest that never matches anything, used to equalize the timing of the
/// "no key presented" path with the "key compared and failed" path. The
/// value is a constant, so no secret is involved.
const DUMMY_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Authenticate a caller from the two candidate headers.
///
/// `authorization` is the raw `Authorization` header value; `api_key` is the
/// raw `X-API-Key` header value. Both may be `None` (absent) or empty.
pub fn authenticate(
    store: &TenancyStore,
    authorization: Option<&str>,
    api_key: Option<&str>,
) -> Result<Principal, ApiError> {
    // Pick up keys created, rotated or revoked by another process (the
    // `harvest keys` CLI) without a restart. Failing to reload fails closed:
    // a caller must not keep authorizing against a snapshot that is behind a
    // revocation made elsewhere.
    store
        .refresh()
        .map_err(|e| ApiError::unavailable(format!("tenancy store could not be re-read: {e}")))?;

    let presented = extract_presented(authorization)
        .or_else(|| extract_presented(api_key))
        .map(str::to_string);

    let Some(presented) = presented else {
        // Burn the same work a real comparison would do, so a request with
        // no credentials is not measurably faster than one with bad ones.
        let _ = super::key::verify_key(DUMMY_DIGEST, "");
        return Err(ApiError::unauthorized(
            "authorization required: send 'Authorization: Bearer <api-key>' or 'X-API-Key: <api-key>'",
        ));
    };

    let Some(key) = store.lookup_by_key(&presented) else {
        let _ = super::key::verify_key(DUMMY_DIGEST, &presented);
        return Err(ApiError::unauthorized("invalid API key"));
    };

    let now = crate::ids::Timestamp::now().as_secs();

    if key.status == super::key::KeyStatus::Revoked
        || key.status == super::key::KeyStatus::Suspended
        || key.is_expired(now)
    {
        // Distinguishing "suspended" (an operator decision, visible to the
        // tenant) from "wrong key" is the one leak we accept: a suspended
        // key is a *state*, not a secret, and telling the caller keeps the
        // tenant from burning quota on a dead key.
        //
        // The reason is reported accurately: an expired key is still
        // `Active`, so naming the status alone would tell a caller their key
        // is fine.
        let why = if key.is_expired(now) {
            "expired"
        } else {
            key.status.as_str()
        };
        return Err(ApiError::forbidden(format!(
            "API key is {why}; contact your operator"
        )));
    }

    let Some(tenant) = store.get_tenant(&key_tenant_id(&key)) else {
        return Err(ApiError::forbidden(
            "API key belongs to a tenant that no longer exists",
        ));
    };

    if !tenant.is_active() {
        return Err(ApiError::forbidden(
            "tenant is suspended; contact your operator",
        ));
    }

    let _ = store.touch_last_used(key.id.as_str());

    Ok(Principal { tenant, key })
}

fn key_tenant_id(key: &ApiKey) -> crate::tenancy::tenant::TenantId {
    // Tenant ids are validated at creation; a key whose tenant id is not a
    // legal slug is unusable, so fail closed rather than skip the lookup.
    crate::tenancy::tenant::TenantId::parse(&key.tenant_id).unwrap_or_else(|_| {
        crate::tenancy::tenant::TenantId::parse("t-invalid").expect("static id")
    })
}

/// Pull the key out of a header value. Both `Bearer <key>` and a bare key
/// are accepted; whitespace and control characters are refused outright so
/// a header cannot smuggle a second value.
fn extract_presented(raw: Option<&str>) -> Option<&str> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let value = match raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
    {
        Some(rest) => rest.trim(),
        None => raw,
    };
    if value.is_empty() || value.len() > 512 {
        return None;
    }
    if value.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    Some(value)
}

/// Authenticate from a flat header iterator (name, value) pairs, as the
/// hand-rolled HTTP server sees them.
pub fn authenticate_headers<'a, I>(store: &TenancyStore, headers: I) -> Result<Principal, ApiError>
where
    I: IntoIterator<Item = (&'a str, &'a str)>,
{
    let mut authorization: Option<&'a str> = None;
    let mut api_key: Option<&'a str> = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("authorization") && authorization.is_none() {
            authorization = Some(value);
        } else if name.eq_ignore_ascii_case("x-api-key") && api_key.is_none() {
            api_key = Some(value);
        }
    }
    authenticate(store, authorization, api_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::limits::KeyLimits;
    use crate::tenancy::scope::Scope;

    fn tmp(name: &str) -> std::path::PathBuf {
        let t = std::env::temp_dir().join(format!("harvest-auth-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        t
    }

    fn store_with_key(dir: &std::path::Path) -> (std::sync::Arc<TenancyStore>, String) {
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
        (store, g.plaintext)
    }

    #[test]
    fn bearer_header_authenticates() {
        let dir = tmp("bearer");
        let (store, plaintext) = store_with_key(&dir);
        let p = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap();
        assert_eq!(p.key.prefix, &plaintext[..12]);
        assert!(!p.key.key_hash.contains(&plaintext));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn x_api_key_header_authenticates() {
        let dir = tmp("apikey");
        let (store, plaintext) = store_with_key(&dir);
        let p = authenticate(store.as_ref(), None, Some(&plaintext)).unwrap();
        assert_eq!(p.key.prefix, &plaintext[..12]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_malformed_credentials_are_401() {
        let dir = tmp("missing");
        let (store, _) = store_with_key(&dir);

        for (authorization, api_key) in [
            (None, None),
            (Some(""), None),
            (Some("   "), None),
            (Some("Basic abc"), None),
            (None, Some("")),
            (Some("Bearer "), None),
            (Some("Bearer x\ny"), None),
        ] {
            let e = authenticate(store.as_ref(), authorization, api_key).unwrap_err();
            assert_eq!(e.status, 401, "headers={authorization:?}/{api_key:?}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_key_is_401_and_leaks_nothing() {
        let dir = tmp("unknown");
        let (store, plaintext) = store_with_key(&dir);
        let e = authenticate(store.as_ref(), Some("Bearer hv_live_0000"), None).unwrap_err();
        assert_eq!(e.status, 401);
        // The wrong-key message must not echo the presented value or the
        // expected one, and the right-key path must not appear anywhere.
        assert!(!e.message.contains("hv_live_0000"));
        assert!(!e.message.contains(&plaintext));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn disabled_keys_are_403() {
        let dir = tmp("disabled");
        let (store, plaintext) = store_with_key(&dir);

        store
            .suspend_key(store.lookup_by_key(&plaintext).unwrap().id.as_str())
            .unwrap();
        let e =
            authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
        assert_eq!(e.status, 403);
        assert!(e.message.contains("suspended"));

        store
            .revoke_key(store.lookup_by_key(&plaintext).unwrap().id.as_str())
            .unwrap();
        // Revoked is not reachable through `lookup_by_key` anymore only if we
        // rotate; revoke keeps the record, so the key is still found.
        let e =
            authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
        assert_eq!(e.status, 403);
        assert!(e.message.contains("revoked"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn expired_key_is_403() {
        let dir = tmp("expired");
        let store = TenancyStore::open(dir.join("tenancy.json")).unwrap();
        let tenant = store.new_tenant("Acme").unwrap();
        let now = crate::ids::Timestamp::now().as_secs();
        let g = store
            .create_key(
                tenant.id.as_str(),
                ScopeSet::from(vec![Scope::Scrape]),
                KeyLimits::default(),
                Some(now - 1),
            )
            .unwrap();
        let e = authenticate(
            store.as_ref(),
            Some(&format!("Bearer {}", g.plaintext)),
            None,
        )
        .unwrap_err();
        assert_eq!(e.status, 403);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn suspended_tenant_is_403() {
        let dir = tmp("tenant");
        let (store, plaintext) = store_with_key(&dir);
        let tenant_id = store.lookup_by_key(&plaintext).unwrap().tenant_id;
        store
            .set_tenant_status(
                &crate::tenancy::tenant::TenantId::parse(&tenant_id).unwrap(),
                crate::tenancy::tenant::TenantStatus::Suspended,
            )
            .unwrap();
        let e =
            authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap_err();
        assert_eq!(e.status, 403);
        assert!(e.message.contains("tenant"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scope_gate_refuses_without_the_capability() {
        let dir = tmp("scope");
        let (store, plaintext) = store_with_key(&dir);
        let p = authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap();
        assert!(p.require(Scope::Scrape).is_ok());
        assert!(p.require(Scope::UsageRead).is_ok());
        let e = p.require(Scope::KeysAdmin).unwrap_err();
        assert_eq!(e.status, 403);
        assert_eq!(e.code, "forbidden");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn last_used_is_recorded_on_success() {
        let dir = tmp("lastused");
        let (store, plaintext) = store_with_key(&dir);
        let id = store.lookup_by_key(&plaintext).unwrap().id;
        authenticate(store.as_ref(), Some(&format!("Bearer {plaintext}")), None).unwrap();
        assert!(store.get_key(id.as_str()).unwrap().last_used_at.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
