//! W3C browser proxy capability generation with explicit attribution.
use super::provider::ProxyEndpoint;
use crate::browser::error::{BrowserError, BrowserResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserEgress {
    pub endpoint_id: String,
    pub provider_id: String,
    pub region: Option<String>,
    pub attached_at_ms: u64,
}

/// Create standards-based proxy capabilities. Credentials are intentionally
/// refused here: browser-authenticated proxy support requires a dedicated,
/// audited driver extension and is not silently emulated with unsafe flags.
pub fn capabilities(endpoint: &ProxyEndpoint) -> BrowserResult<Value> {
    let url = url::Url::parse(&endpoint.url)
        .map_err(|_| BrowserError::InvalidInput("proxy endpoint URL"))?;
    if !matches!(
        url.scheme(),
        "http" | "https" | "socks4" | "socks5" | "socks5h"
    ) || url.host_str().is_none()
    {
        return Err(BrowserError::InvalidInput("proxy endpoint URL"));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || endpoint.username_secret_ref.is_some()
        || endpoint.password_secret_ref.is_some()
    {
        return Err(BrowserError::Unsupported("authenticated browser proxy attachment; use an authorized provider gateway without embedded credentials"));
    }
    let proxy_url = format!(
        "{}:{}",
        url.host_str().unwrap_or_default(),
        url.port().unwrap_or(match url.scheme() {
            "https" => 443,
            "socks4" | "socks5" | "socks5h" => 1080,
            _ => 80,
        })
    );
    Ok(json!({"proxyType":"manual","httpProxy":proxy_url,"sslProxy":proxy_url}))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn endpoint(url: &str) -> ProxyEndpoint {
        ProxyEndpoint {
            id: "p1".into(),
            url: url.into(),
            region: None,
            username_secret_ref: None,
            password_secret_ref: None,
            supports_session_affinity: false,
            supports_region_selection: false,
        }
    }
    #[test]
    fn builds_manual_w3c_capability() {
        let v = capabilities(&endpoint("http://proxy.test:8080")).unwrap();
        assert_eq!(v["proxyType"], "manual");
    }
    #[test]
    fn rejects_embedded_credentials() {
        assert!(capabilities(&endpoint("http://u:p@proxy.test:8080")).is_err());
    }
}
