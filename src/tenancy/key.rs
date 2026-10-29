//! API keys: generation, hashing, verification and the key record.
//!
//! The invariant this module exists to keep: **the plaintext key exists in
//! memory for exactly one call** (the response of key creation or rotation),
//! and only ever leaves the process as that response. What is stored, what
//! is logged, what is hashed into an audit line and what is compared is the
//! SHA-256 of the plaintext — never the plaintext.
//!
//! * Generation uses the OS RNG (`getrandom`), 128 bits of key material.
//! * Comparison is constant-time over the digest, so a timing side channel
//!   cannot learn how many leading characters of a guessed key are correct.
//! * The stored record carries a short, non-secret `prefix` so an operator
//!   can tell two keys apart in a listing without a brute-force search.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::limits::KeyLimits;
use super::scope::ScopeSet;
use super::tenant::random_hex;

/// Hard ceiling for a per-key response body cap. Mirrors the engine's own
/// ceiling so a managed key can never ask for more buffering than a crawl.
pub const MAX_RESPONSE_BYTES_CEILING: u64 = crate::config::MAX_RESPONSE_BYTES_CEILING;

/// Public key prefix. `hv_live_` marks a production key; test keys use the
/// same shape (there is exactly one format, so a key is never ambiguous).
pub const KEY_PREFIX: &str = "hv_live_";
/// Random key material, in hex characters.
pub const KEY_MATERIAL_HEX: usize = 32;
/// Display prefix length (the part shown in listings).
pub const KEY_PREFIX_DISPLAY: usize = 12;
/// Longest key id.
pub const KEY_ID_MAX: usize = 64;

/// Key lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Suspended,
    Revoked,
}

impl KeyStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            KeyStatus::Active => "active",
            KeyStatus::Suspended => "suspended",
            KeyStatus::Revoked => "revoked",
        }
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(KeyStatus::Active),
            "suspended" => Ok(KeyStatus::Suspended),
            "revoked" => Ok(KeyStatus::Revoked),
            other => Err(format!(
                "unknown key status `{other}` (allowed: active, suspended, revoked)"
            )),
        }
    }
}

impl fmt::Display for KeyStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated key id (`key_<hex>`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct KeyId(String);

impl KeyId {
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() || s.len() > KEY_ID_MAX || !s.starts_with("key_") {
            return Err(format!("key id must look like `key_<hex>` (got `{s}`)"));
        }
        let tail = &s[4..];
        if tail.is_empty() || tail.len() > 32 || !tail.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("key id `{s}` must carry 1..=32 hex characters"));
        }
        Ok(Self(s.to_string()))
    }

    pub fn new_random() -> Self {
        Self(format!("key_{}", random_hex(8)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for KeyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// SHA-256 of a key, lowercase hex. This is what is persisted and compared.
pub fn hash_key(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Constant-time comparison of two byte strings. Length is folded into the
/// accumulator so a shorter guess does not short-circuit the loop.
pub(crate) fn timing_eq(a: &[u8], b: &[u8]) -> bool {
    let n = a.len().max(b.len());
    let mut diff = a.len() ^ b.len();
    for i in 0..n {
        let x = *a.get(i).unwrap_or(&0);
        let y = *b.get(i).unwrap_or(&0);
        diff |= (x ^ y) as usize;
    }
    diff == 0
}

/// Constant-time verification of a presented key against a stored digest.
pub fn verify_key(stored_hash_hex: &str, presented: &str) -> bool {
    let stored = match hex_decode(stored_hash_hex) {
        Some(v) => v,
        None => return false,
    };
    let presented = hash_key(presented);
    let want = match hex_decode(&presented) {
        Some(v) => v,
        None => return false,
    };
    timing_eq(&stored, &want)
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || s.len() > 128 {
        return None;
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len() / 2);
    let mut i = 0;
    while i < b.len() {
        let hi = (b[i] as char).to_digit(16)?;
        let lo = (b[i + 1] as char).to_digit(16)?;
        out.push(hi as u8 * 16 + lo as u8);
        i += 2;
    }
    Some(out)
}

/// A freshly minted key: the record to persist, plus the plaintext that is
/// shown to the caller **once** and then dropped.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratedKey {
    pub key: ApiKey,
    pub plaintext: String,
}

/// The persisted key record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKey {
    pub id: KeyId,
    pub tenant_id: String,
    /// SHA-256 of the plaintext, lowercase hex. Never the plaintext.
    pub key_hash: String,
    /// Non-secret display prefix (`hv_live_ab12cd`).
    pub prefix: String,
    pub scopes: ScopeSet,
    pub limits: KeyLimits,
    pub status: KeyStatus,
    /// Epoch seconds; `None` = never expires.
    pub expires_at: Option<i64>,
    /// Epoch seconds.
    pub created_at: i64,
    /// Epoch seconds of the last accepted request (throttled update).
    pub last_used_at: Option<i64>,
}

impl ApiKey {
    /// Generate a fresh key for `tenant_id`. `scopes`/`limits` are
    /// validated before anything is written.
    pub fn create(
        tenant_id: &str,
        scopes: ScopeSet,
        limits: KeyLimits,
        expires_at: Option<i64>,
    ) -> Result<GeneratedKey, String> {
        limits.validate()?;
        let plaintext = format!("{}{}", KEY_PREFIX, random_hex(KEY_MATERIAL_HEX / 2));
        let id = KeyId::new_random();
        let key = ApiKey {
            key_hash: hash_key(&plaintext),
            prefix: plaintext
                .chars()
                .take(KEY_PREFIX_DISPLAY)
                .collect::<String>(),
            id,
            tenant_id: tenant_id.to_string(),
            scopes,
            limits,
            status: KeyStatus::Active,
            expires_at,
            created_at: crate::ids::Timestamp::now().as_secs(),
            last_used_at: None,
        };
        Ok(GeneratedKey { key, plaintext })
    }

    /// Rotate: mint a new record carrying the same tenant/scopes/limits,
    /// leaving the old one revoked. The new plaintext is returned to the
    /// caller exactly once.
    pub fn rotate(existing: &ApiKey, expires_at: Option<i64>) -> Result<GeneratedKey, String> {
        Self::create(
            &existing.tenant_id,
            existing.scopes.clone(),
            existing.limits.clone(),
            expires_at,
        )
    }

    /// A key is usable when it is active and has not expired. Suspended and
    /// revoked keys never authenticate.
    pub fn is_usable(&self, now: i64) -> bool {
        self.status == KeyStatus::Active && self.expires_at.is_none_or(|e| e > now)
    }

    pub fn is_expired(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|e| e <= now)
    }

    pub fn allows(&self, needed: super::scope::Scope) -> bool {
        self.scopes.contains(needed) || self.scopes.contains(super::scope::Scope::KeysAdmin)
    }
}

/// Generate a key, returning the record and the one-time plaintext.
pub fn generate_key(
    tenant_id: &str,
    scopes: ScopeSet,
    limits: KeyLimits,
    expires_at: Option<i64>,
) -> Result<GeneratedKey, String> {
    ApiKey::create(tenant_id, scopes, limits, expires_at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::scope::{Scope, ScopeSet};

    fn key() -> ApiKey {
        generate_key(
            "t-1",
            ScopeSet::from(vec![Scope::Scrape]),
            KeyLimits::default(),
            None,
        )
        .unwrap()
        .key
    }

    #[test]
    fn plaintext_is_never_the_stored_hash() {
        let g = generate_key("t-1", ScopeSet::default(), KeyLimits::default(), None).unwrap();
        assert!(g.plaintext.starts_with(KEY_PREFIX));
        assert_eq!(g.plaintext.len(), KEY_PREFIX.len() + KEY_MATERIAL_HEX);
        assert_ne!(g.key.key_hash, g.plaintext);
        assert_ne!(g.key.key_hash.len(), g.plaintext.len());
        assert_eq!(g.key.key_hash.len(), 64);
        assert_eq!(g.key.prefix, &g.plaintext[..KEY_PREFIX_DISPLAY]);
    }

    #[test]
    fn verification_accepts_only_the_right_key() {
        let g = generate_key("t-1", ScopeSet::default(), KeyLimits::default(), None).unwrap();
        assert!(verify_key(&g.key.key_hash, &g.plaintext));
        assert!(!verify_key(
            &g.key.key_hash,
            "hv_live_00000000000000000000000000000000"
        ));
        assert!(!verify_key(&g.key.key_hash, &g.plaintext[..10]));
        assert!(!verify_key("not-hex", &g.plaintext));
        assert!(!verify_key(&g.key.key_hash, ""));
    }

    #[test]
    fn two_keys_have_distinct_hashes() {
        let a = generate_key("t-1", ScopeSet::default(), KeyLimits::default(), None).unwrap();
        let b = generate_key("t-1", ScopeSet::default(), KeyLimits::default(), None).unwrap();
        assert_ne!(a.plaintext, b.plaintext);
        assert_ne!(a.key.key_hash, b.key.key_hash);
        assert_ne!(a.key.id, b.key.id);
    }

    #[test]
    fn expiry_and_status_gate_usability() {
        let now = crate::ids::Timestamp::now().as_secs();
        let k = key();
        assert!(k.is_usable(now));

        let expired = generate_key(
            "t-1",
            ScopeSet::default(),
            KeyLimits::default(),
            Some(now - 1),
        )
        .unwrap()
        .key;
        assert!(expired.is_expired(now));
        assert!(!expired.is_usable(now));
        assert!(expired.is_usable(now - 10));

        let mut suspended = key();
        suspended.status = KeyStatus::Suspended;
        assert!(!suspended.is_usable(now));
    }

    #[test]
    fn id_shape_is_enforced() {
        let id = KeyId::new_random();
        assert!(KeyId::parse(id.as_str()).is_ok());
        for bad in [
            "",
            "abc",
            "key",
            "key_",
            "key_zz",
            "key_/x",
            "key_12345678901234567890123456789012345",
        ] {
            assert!(KeyId::parse(bad).is_err(), "id {bad:?} must be rejected");
        }
    }

    #[test]
    fn limits_are_validated_at_creation() {
        let bad = KeyLimits {
            rps: -1.0,
            ..KeyLimits::default()
        };
        assert!(generate_key("t-1", ScopeSet::default(), bad, None).is_err());
    }

    #[test]
    fn rotation_keeps_identity_and_revokes_nothing_by_itself() {
        let original = key();
        let rotated = ApiKey::rotate(&original, None).unwrap();
        assert_eq!(rotated.key.tenant_id, original.tenant_id);
        assert_eq!(rotated.key.scopes, original.scopes);
        assert_eq!(rotated.key.limits, original.limits);
        assert_ne!(rotated.key.id, original.id);
        assert!(verify_key(&rotated.key.key_hash, &rotated.plaintext));
    }

    #[test]
    fn scope_checks_are_capability_based() {
        let k = generate_key(
            "t-1",
            ScopeSet::from(vec![Scope::Scrape]),
            KeyLimits::default(),
            None,
        )
        .unwrap()
        .key;
        assert!(k.allows(Scope::Scrape));
        assert!(!k.allows(Scope::UsageRead));
        assert!(!k.allows(Scope::KeysAdmin));
    }
}
