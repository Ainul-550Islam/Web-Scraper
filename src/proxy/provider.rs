//! Provider-neutral proxy endpoint contract. Implementations return bounded
//! leases and report health; refusal handling remains owned by crawl policy.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyRequest {
    pub target_id: String,
    pub session_key: Option<String>,
    pub preferred_region: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyEndpoint {
    pub id: String,
    pub url: String,
    pub region: Option<String>,
    pub username_secret_ref: Option<String>,
    pub password_secret_ref: Option<String>,
    pub supports_session_affinity: bool,
    pub supports_region_selection: bool,
}
impl fmt::Debug for ProxyEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyEndpoint")
            .field("id", &self.id)
            .field("url", &redacted_url(&self.url))
            .field("region", &self.region)
            .field("username_secret_ref", &self.username_secret_ref)
            .field("password_secret_ref", &self.password_secret_ref)
            .field("supports_session_affinity", &self.supports_session_affinity)
            .field("supports_region_selection", &self.supports_region_selection)
            .finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub healthy: bool,
    pub latency_ms: Option<u64>,
    pub failures: u64,
    pub refusals: u64,
    pub checked_at_ms: u64,
    pub message: Option<String>,
}
#[derive(Clone, Debug)]
pub struct ProviderError {
    pub code: String,
    pub safe_message: String,
}
impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "proxy provider {}: {}",
            self.code,
            self.safe_message.chars().take(256).collect::<String>()
        )
    }
}
impl std::error::Error for ProviderError {}

pub trait ProxyProvider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn acquire(&self, request: &ProxyRequest) -> Result<ProxyEndpoint, ProviderError>;
    fn report(&self, endpoint_id: &str, success: bool, refusal: bool, latency_ms: u64);
    fn health(&self) -> ProviderHealth;
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub regions: Vec<String>,
    pub session_affinity: bool,
    pub authenticated_endpoints: bool,
    pub leases: bool,
    pub max_concurrency: Option<usize>,
}

pub fn redacted_url(input: &str) -> String {
    match url::Url::parse(input) {
        Ok(mut u) => {
            if !u.username().is_empty() {
                let _ = u.set_username("[REDACTED]");
            }
            if u.password().is_some() {
                let _ = u.set_password(Some("[REDACTED]"));
            }
            u.to_string()
        }
        Err(_) => "[invalid proxy URL]".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn debug_redacts_proxy_password() {
        let p = ProxyEndpoint {
            id: "p".into(),
            url: "http://alice:secret@proxy.test:8080".into(),
            region: None,
            username_secret_ref: None,
            password_secret_ref: None,
            supports_session_affinity: false,
            supports_region_selection: false,
        };
        let s = format!("{p:?}");
        assert!(!s.contains("secret"));
        assert!(s.contains("REDACTED"));
    }
}
