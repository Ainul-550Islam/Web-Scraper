//! Per-key resource limits and the validation bounds that keep them honest.
//!
//! A key carries its own pacing, concurrency and quota so that one tenant's
//! burst cannot starve another's politeness budget — and so that a limit
//! can never be *raised* by a client. Every value here is validated against
//! a hard bound at key-creation and at config load; the bounds are part of
//! the contract, not a suggestion.
//!
//! Defaults are deliberately conservative: a fresh key is a polite customer,
//! not a crawler. Operators raise them explicitly per key.

use serde::{Deserialize, Serialize};

use super::scope::ScopeSet;

/// Hard ceiling for a per-key monthly quota (requests/month). Above this a
/// single tenant could exhaust the engine's whole politeness budget.
pub const MONTHLY_QUOTA_CEILING: u64 = 100_000_000;
/// Hard ceiling for a per-key burst.
pub const BURST_CEILING: u32 = 10_000;
/// Hard ceiling for per-key concurrency.
pub const CONCURRENCY_CEILING: u32 = 64;
/// Hard ceiling for the per-key request rate.
pub const RPS_CEILING: f64 = 1_000.0;
/// Maximum number of entries in a per-key domain allowlist.
pub const ALLOWED_DOMAINS_MAX: usize = 256;
/// Longest single allowlist entry (a registrable host label).
pub const ALLOWED_DOMAIN_MAX: usize = 253;

/// Every limit one managed key carries. Serialization is the wire shape used
/// by `POST /v1/admin/keys`, `GET /v1/admin/keys` and `LimitsEditor`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyLimits {
    /// Sustained requests per second for this key (token bucket refill).
    pub rps: f64,
    /// Burst capacity of the token bucket.
    pub burst: u32,
    /// Max jobs this key may have in flight simultaneously.
    pub max_concurrent_jobs: u32,
    /// Monthly request quota; `0` = unlimited.
    pub monthly_quota: u64,
    /// Hard cap on the response body this key may receive (bytes).
    pub max_response_bytes: u64,
    /// Domain allowlist. Empty = no restriction beyond the SSRF guard.
    pub allowed_domains: Vec<String>,
    /// Scopes granted to the key (normalised, sorted, deduplicated).
    pub scopes: ScopeSet,
}

impl Default for KeyLimits {
    fn default() -> Self {
        Self {
            rps: 5.0,
            burst: 10,
            max_concurrent_jobs: 2,
            monthly_quota: 100_000,
            max_response_bytes: 16 * 1024 * 1024,
            allowed_domains: Vec::new(),
            scopes: ScopeSet::from(vec![super::scope::Scope::Scrape]),
        }
    }
}

impl KeyLimits {
    /// Bounds-check every field. Called on key creation, on limits edits and
    /// on config load for the default limits — so a typo cannot create a key
    /// that the rate limiter would mis-handle.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.rps.is_finite() && 0.0 < self.rps && self.rps <= RPS_CEILING) {
            return Err(format!(
                "limits.rps must be finite and in (0, {RPS_CEILING}] (got {})",
                self.rps
            ));
        }
        if self.burst == 0 || self.burst > BURST_CEILING {
            return Err(format!(
                "limits.burst must be in 1..={BURST_CEILING} (got {})",
                self.burst
            ));
        }
        if self.max_concurrent_jobs == 0 || self.max_concurrent_jobs > CONCURRENCY_CEILING {
            return Err(format!(
                "limits.max_concurrent_jobs must be in 1..={CONCURRENCY_CEILING} (got {})",
                self.max_concurrent_jobs
            ));
        }
        if self.monthly_quota > MONTHLY_QUOTA_CEILING {
            return Err(format!(
                "limits.monthly_quota must be <= {MONTHLY_QUOTA_CEILING} (got {})",
                self.monthly_quota
            ));
        }
        if self.max_response_bytes < 4096
            || self.max_response_bytes > super::key::MAX_RESPONSE_BYTES_CEILING
        {
            return Err(format!(
                "limits.max_response_bytes must be in 4096..={} (got {})",
                super::key::MAX_RESPONSE_BYTES_CEILING,
                self.max_response_bytes
            ));
        }
        if self.allowed_domains.len() > ALLOWED_DOMAINS_MAX {
            return Err(format!(
                "limits.allowed_domains allows at most {ALLOWED_DOMAINS_MAX} entries (got {})",
                self.allowed_domains.len()
            ));
        }
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for host in &self.allowed_domains {
            let host = host.trim();
            if host.is_empty()
                || host.len() > ALLOWED_DOMAIN_MAX
                || host.contains('/')
                || host.contains(':')
                || host.starts_with('.')
                || host.ends_with('.')
                || host.contains("..")
                || !host
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            {
                return Err(format!(
                    "limits.allowed_domains entry `{host}` must be a bare host label \
                     (no scheme, port, path or leading/trailing dot)"
                ));
            }
            if !seen.insert(host) {
                return Err(format!("duplicate limits.allowed_domains entry `{host}`"));
            }
        }
        Ok(())
    }

    /// Domain allowlist check against a request host (lower-cased registrable
    /// host, no port). Empty allowlist = allow all (the SSRF guard still
    /// applies, and the politeness layer paces every host).
    pub fn allows_domain(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        if host.is_empty() {
            return false;
        }
        if self.allowed_domains.is_empty() {
            return true;
        }
        self.allowed_domains.iter().any(|allowed| {
            let allowed = allowed.trim_end_matches('.').to_ascii_lowercase();
            host == allowed || host.ends_with(&format!(".{allowed}"))
        })
    }

    /// Retry-After hint for a refused rate-limited request. Bounded so a
    /// misconfigured `rps` cannot ask a client to wait a week.
    pub fn retry_after_secs(&self) -> u64 {
        let wait = 1.0 / self.rps;
        wait.ceil().clamp(1.0, 3600.0) as u64
    }

    /// Whether the key may spend a unit of its monthly quota.
    pub fn quota_enabled(&self) -> bool {
        self.monthly_quota > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tenancy::scope::Scope;

    fn limits() -> KeyLimits {
        KeyLimits::default()
    }

    #[test]
    fn defaults_are_polite_and_valid() {
        let l = limits();
        l.validate().expect("default limits must validate");
        assert!(l.allows_domain("example.com"));
        assert!(l.allows_domain("sub.example.com"));
        assert!(!l.allows_domain(""));
    }

    #[test]
    fn empty_allowlist_allows_every_host() {
        let mut l = limits();
        l.allowed_domains = Vec::new();
        assert!(l.allows_domain("anything.test"));
    }

    #[test]
    fn allowlist_is_host_scoped_not_substring() {
        let mut l = limits();
        l.allowed_domains = vec!["example.com".into()];
        assert!(l.allows_domain("example.com"));
        assert!(l.allows_domain("www.example.com"));
        assert!(l.allows_domain("example.com."));
        assert!(!l.allows_domain("notexample.com"));
        assert!(!l.allows_domain("example.com.evil.test"));
        assert!(!l.allows_domain("example.com:8080"));
        assert!(!l.allows_domain("https://example.com"));
    }

    #[test]
    fn allowlist_entry_bounds_are_enforced() {
        for bad in [
            "".to_string(),
            "/path".to_string(),
            "host:80".to_string(),
            ".example.com".to_string(),
            "example.com.".to_string(),
            "a..b".to_string(),
            "exa mple.com".to_string(),
            "/".repeat(ALLOWED_DOMAIN_MAX + 1),
        ] {
            let mut l = limits();
            l.allowed_domains = vec![bad.clone()];
            assert!(l.validate().is_err(), "entry {bad:?} must be rejected");
        }
    }

    #[test]
    fn numeric_bounds_are_enforced() {
        let mut l = limits();
        l.rps = 0.0;
        assert!(l.validate().is_err());
        l.rps = -1.0;
        assert!(l.validate().is_err());
        l.rps = RPS_CEILING + 1.0;
        assert!(l.validate().is_err());

        l = limits();
        l.burst = 0;
        assert!(l.validate().is_err());
        l.burst = BURST_CEILING + 1;
        assert!(l.validate().is_err());

        l = limits();
        l.max_concurrent_jobs = 0;
        assert!(l.validate().is_err());
        l.max_concurrent_jobs = CONCURRENCY_CEILING + 1;
        assert!(l.validate().is_err());

        l = limits();
        l.monthly_quota = MONTHLY_QUOTA_CEILING + 1;
        assert!(l.validate().is_err());

        l = limits();
        l.max_response_bytes = 1024;
        assert!(l.validate().is_err());
    }

    #[test]
    fn retry_after_is_bounded() {
        let mut l = limits();
        l.rps = 0.5;
        assert_eq!(l.retry_after_secs(), 2);
        l.rps = 1000.0;
        assert_eq!(l.retry_after_secs(), 1);
        l.rps = 0.001;
        assert_eq!(l.retry_after_secs(), 1000);
        l.rps = 1.0 / 7200.0;
        assert_eq!(l.retry_after_secs(), 3600);
    }

    #[test]
    fn scopes_round_trip_through_the_wire_type() {
        let l = KeyLimits {
            scopes: ScopeSet::from(vec![Scope::UsageRead, Scope::Scrape]),
            ..KeyLimits::default()
        };
        let v = serde_json::to_value(&l).unwrap();
        assert_eq!(v["scopes"], serde_json::json!(["scrape", "usage_read"]));
        let back: KeyLimits = serde_json::from_value(v).unwrap();
        assert_eq!(back, l);
    }

    #[test]
    fn unknown_fields_are_refused() {
        let raw = serde_json::json!({ "rps": 1.0, "surprise": true });
        assert!(serde_json::from_value::<KeyLimits>(raw).is_err());
    }
}
