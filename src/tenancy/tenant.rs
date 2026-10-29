//! Tenants: the unit that owns keys, quota and job visibility.
//!
//! A tenant is an operator-defined account. Everything a key can do is
//! scoped to the tenant that owns it: quota is per tenant's keys, job
//! listings are per tenant, and one tenant can never read another's job id
//! (a foreign id answers 404, not 403 — no existence oracle).

use std::fmt;

use serde::{Deserialize, Serialize};

/// Longest tenant id (slug shape: `[a-z0-9][a-z0-9._-]{0,63}`).
pub const TENANT_ID_MAX: usize = 64;
/// Longest tenant display name.
pub const TENANT_NAME_MAX: usize = 200;

/// Tenant lifecycle. A suspended tenant keeps its keys but stops being able
/// to act; revocation is a key-level decision (see [`super::key::KeyStatus`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantStatus {
    Active,
    Suspended,
}

impl TenantStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TenantStatus::Active => "active",
            TenantStatus::Suspended => "suspended",
        }
    }
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "active" => Ok(TenantStatus::Active),
            "suspended" => Ok(TenantStatus::Suspended),
            other => Err(format!(
                "unknown tenant status `{other}` (allowed: active, suspended)"
            )),
        }
    }
}

impl fmt::Display for TenantStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Validated tenant id. The shape is the same class the rest of the engine
/// uses for identifiers that end up in filenames and log lines: lowercase
/// ASCII, no path or key separators, never `..`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TenantId(String);

impl TenantId {
    pub fn parse(s: &str) -> Result<Self, String> {
        if s.is_empty() || s.len() > TENANT_ID_MAX {
            return Err(format!("tenant id must be 1..={TENANT_ID_MAX} bytes"));
        }
        let mut chars = s.chars();
        let first = chars.next().expect("checked non-empty");
        if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
            return Err(format!(
                "tenant id `{s}` must start with a lowercase letter or digit"
            ));
        }
        for c in chars {
            if !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-')) {
                return Err(format!(
                    "tenant id `{s}` has an illegal byte `{c}` (allowed: a-z 0-9 . _ -)"
                ));
            }
        }
        if s.contains('/')
            || s.contains('\\')
            || s.contains(':')
            || s.contains("..")
            || s.ends_with('.')
        {
            return Err(format!(
                "tenant id `{s}` must not contain path or key separators"
            ));
        }
        Ok(Self(s.to_string()))
    }

    pub fn new_random() -> Self {
        Self(format!("t-{}", random_hex(12)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// OS-backed random hex. Ids are log join keys, not capability tokens — the
/// secret material is the API key itself, and that uses the same RNG.
pub(crate) fn random_hex(bytes: usize) -> String {
    let mut raw = vec![0u8; bytes];
    if getrandom::fill(&mut raw).is_err() {
        // Fail loudly rather than hand out predictable ids.
        panic!("operating-system randomness is unavailable");
    }
    let mut out = String::with_capacity(bytes * 2);
    for b in raw {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// One tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub status: TenantStatus,
    /// Epoch seconds.
    pub created_at: i64,
}

impl Tenant {
    /// Create a tenant with a fresh id. `name` is validated; an operator who
    /// wants a specific id uses [`Tenant::with_id`].
    pub fn create(name: &str) -> Result<Self, String> {
        Self::with_id(TenantId::new_random(), name)
    }

    pub fn with_id(id: TenantId, name: &str) -> Result<Self, String> {
        let name = name.trim();
        if name.is_empty() || name.len() > TENANT_NAME_MAX {
            return Err(format!(
                "tenant name must be 1..={TENANT_NAME_MAX} bytes after trimming"
            ));
        }
        if name
            .bytes()
            .any(|b| b.is_ascii_control() || b == b'\n' || b == b'\r')
        {
            return Err("tenant name must not contain control characters".to_string());
        }
        Ok(Self {
            id,
            name: name.to_string(),
            status: TenantStatus::Active,
            created_at: crate::ids::Timestamp::now().as_secs(),
        })
    }

    pub fn is_active(&self) -> bool {
        self.status == TenantStatus::Active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ids_are_well_shaped_and_unique() {
        let a = TenantId::new_random();
        let b = TenantId::new_random();
        assert_ne!(a, b);
        assert!(TenantId::parse(a.as_str()).is_ok());
        assert!(a.as_str().starts_with("t-"));
    }

    #[test]
    fn id_shape_is_enforced() {
        for bad in [
            "",
            "Abc",
            "/etc",
            "a/b",
            "a:b",
            "..",
            "a..b",
            "trail.",
            &"x".repeat(TENANT_ID_MAX + 1),
            "unsafe\n",
        ] {
            assert!(TenantId::parse(bad).is_err(), "id {bad:?} must be rejected");
        }
        for good in ["tenant", "t-1", "acme_2", "a.b-c"] {
            assert!(
                TenantId::parse(good).is_ok(),
                "id {good:?} must be accepted"
            );
        }
    }

    #[test]
    fn tenant_creation_validates_the_name() {
        let t = Tenant::create("Acme Data Co").unwrap();
        assert!(t.is_active());
        assert_eq!(t.name, "Acme Data Co");
        assert!(t.created_at > 0);

        for bad in ["", "   ", &"n".repeat(TENANT_NAME_MAX + 1), "bad\nname"] {
            assert!(
                Tenant::create(bad).is_err(),
                "name {bad:?} must be rejected"
            );
        }
    }

    #[test]
    fn status_vocabulary_is_closed() {
        assert_eq!(TenantStatus::parse("active").unwrap(), TenantStatus::Active);
        assert_eq!(
            TenantStatus::parse("suspended").unwrap(),
            TenantStatus::Suspended
        );
        assert!(TenantStatus::parse("deleted").is_err());
    }

    #[test]
    fn wire_round_trip_is_exact() {
        let t = Tenant::create("Acme").unwrap();
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["status"], "active");
        let back: Tenant = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }
}
