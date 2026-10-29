//! Scope vocabulary for managed API keys.
//!
//! A scope is a *capability*, not a resource: it answers "may this key do
//! this kind of work", and the answer is enforced at the handler boundary,
//! never by convention. The vocabulary is closed — an unknown scope string
//! in a request is a 400, not a silently ignored capability.
//!
//! Design rules:
//! * Scopes are additive and additive-only: a key granted `scrape` may call
//!   `POST /v1/scrape`; it does not inherit `jobs_write`, `usage_read` or
//!   `keys_admin` because those answer different questions.
//! * Granting `keys_admin` on a *customer* key would let a tenant mint keys
//!   for itself; the operator surface (`/v1/admin/*`) therefore requires the
//!   admin bearer token, and `keys_admin` is only meaningful for a key that
//!   an operator deliberately created.
//! * `parse` is total over the wire vocabulary and rejects everything else
//!   (`deny_unknown_fields` on the request struct already refuses unknown
//!   keys; this refuses unknown *values*).

use serde::{Deserialize, Serialize};

/// The closed scope vocabulary. Order is declaration order and is stable in
/// every serialization (scopes serialize as their wire name).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// `POST /v1/scrape`, `POST /v1/jobs`, cancel.
    Scrape,
    /// `GET /v1/jobs`, `GET /v1/jobs/:id` (own tenant's jobs only).
    JobsRead,
    /// `POST /v1/jobs/:id/cancel` (own tenant's jobs only).
    JobsWrite,
    /// `GET /v1/usage` for the calling key.
    UsageRead,
    /// Key administration: create/rotate/revoke/suspend keys.
    KeysAdmin,
}

impl Scope {
    pub const ALL: &'static [Scope] = &[
        Scope::Scrape,
        Scope::JobsRead,
        Scope::JobsWrite,
        Scope::UsageRead,
        Scope::KeysAdmin,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Scrape => "scrape",
            Scope::JobsRead => "jobs_read",
            Scope::JobsWrite => "jobs_write",
            Scope::UsageRead => "usage_read",
            Scope::KeysAdmin => "keys_admin",
        }
    }

    /// Closed vocabulary parse. Unknown values are an error: a scope that
    /// silently dropped out of a key would turn a granted capability into a
    /// quiet no-op.
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "scrape" => Ok(Scope::Scrape),
            "jobs_read" => Ok(Scope::JobsRead),
            "jobs_write" => Ok(Scope::JobsWrite),
            "usage_read" => Ok(Scope::UsageRead),
            "keys_admin" => Ok(Scope::KeysAdmin),
            other => Err(format!(
                "unknown scope `{other}` (allowed: scrape, jobs_read, jobs_write, usage_read, keys_admin)"
            )),
        }
    }

    /// Capability check. `keys_admin` is the operator grant: it implies every
    /// capability because an operator acting through the key exercises all of
    /// them. Every other scope grants exactly one capability.
    pub fn allows(self, needed: Scope) -> bool {
        self == needed || self == Scope::KeysAdmin
    }

    /// True when the key may read job state.
    pub fn covers_jobs(self) -> bool {
        self == Scope::JobsRead || self == Scope::KeysAdmin
    }

    /// True when the key may mutate job state (cancel).
    pub fn covers_job_write(self) -> bool {
        self == Scope::JobsWrite || self == Scope::KeysAdmin
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Deduplicated, order-stable scope list parsed from a wire request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScopeSet(Vec<Scope>);

impl ScopeSet {
    /// Closed-vocabulary parse with deduplication. An empty list is allowed
    /// (a key with no capabilities can authenticate but cannot act).
    pub fn parse_list(raw: &[String]) -> Result<Self, String> {
        let mut set: Vec<Scope> = Vec::with_capacity(raw.len());
        for item in raw {
            let scope = Scope::parse(item)?;
            if !set.contains(&scope) {
                set.push(scope);
            }
        }
        set.sort();
        Ok(Self(set))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Scope> {
        self.0.iter()
    }

    pub fn contains(&self, scope: Scope) -> bool {
        self.0.contains(&scope)
    }

    /// Aggregate check: true when every capability in `needed` is granted.
    pub fn allows(&self, needed: &[Scope]) -> bool {
        needed.iter().all(|n| self.0.iter().any(|g| g.allows(*n)))
    }
}

impl<'a> IntoIterator for &'a ScopeSet {
    type Item = &'a Scope;
    type IntoIter = std::slice::Iter<'a, Scope>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Vec<Scope>> for ScopeSet {
    fn from(v: Vec<Scope>) -> Self {
        let mut set = v;
        set.sort();
        set.dedup();
        Self(set)
    }
}

impl From<&[Scope]> for ScopeSet {
    fn from(v: &[Scope]) -> Self {
        Self(v.to_vec())
    }
}

impl Serialize for ScopeSet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ScopeSet {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw: Vec<String> = Vec::deserialize(d)?;
        ScopeSet::parse_list(&raw).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vocabulary_is_closed_and_exact() {
        for (raw, want) in [
            ("scrape", Scope::Scrape),
            ("jobs_read", Scope::JobsRead),
            ("jobs_write", Scope::JobsWrite),
            ("usage_read", Scope::UsageRead),
            ("keys_admin", Scope::KeysAdmin),
        ] {
            assert_eq!(Scope::parse(raw).unwrap(), want, "raw={raw}");
            assert_eq!(want.as_str(), raw);
        }
        for bad in ["", "admin", "Scrape", "scrape ", "jobs"] {
            assert!(Scope::parse(bad).is_err(), "raw={bad} must be rejected");
        }
    }

    #[test]
    fn scopes_do_not_imply_other_scopes() {
        let set = ScopeSet::from(vec![Scope::Scrape]);
        assert!(set.allows(&[Scope::Scrape]));
        assert!(!set.allows(&[Scope::JobsWrite]));
        assert!(!set.allows(&[Scope::UsageRead]));
        assert!(!set.allows(&[Scope::JobsRead]));
    }

    #[test]
    fn keys_admin_acts_through_the_key() {
        let set = ScopeSet::from(vec![Scope::KeysAdmin]);
        assert!(set.allows(&[Scope::KeysAdmin]));
        assert!(set.allows(&[Scope::JobsWrite]));
        assert!(set.allows(&[Scope::UsageRead]));
        assert!(set.allows(&[Scope::JobsRead]));
    }

    #[test]
    fn parse_list_dedupes_and_sorts() {
        let set =
            ScopeSet::parse_list(&["usage_read".into(), "scrape".into(), "scrape".into()]).unwrap();
        assert_eq!(set.len(), 2);
        assert_eq!(
            set.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            vec!["scrape", "usage_read"]
        );
        assert!(ScopeSet::parse_list(&["nope".into()]).is_err());
    }

    #[test]
    fn wire_round_trip_keeps_names() {
        let set = ScopeSet::parse_list(&["jobs_write".into(), "scrape".into()]).unwrap();
        let v = serde_json::to_value(&set).unwrap();
        assert_eq!(v, serde_json::json!(["scrape", "jobs_write"]));
        let back: ScopeSet = serde_json::from_value(v).unwrap();
        assert_eq!(back, set);
    }
}
