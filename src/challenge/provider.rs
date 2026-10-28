//! Provider-neutral, auditable challenge handling interface.
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    pub request_id: String,
    pub domain: String,
    pub challenge_kind: String,
    pub page_url: String,
    pub site_key: Option<String>,
    pub authorized_scope: bool,
    pub timeout_ms: u64,
    pub target_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderResult {
    pub request_id: String,
    pub status: ProviderStatus,
    pub cost_microusd: u64,
    pub elapsed_ms: u64,
    pub reference: Option<String>,
    pub safe_message: Option<String>,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Accepted,
    Completed,
    Unavailable,
    Rejected,
    TimedOut,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub challenge_kinds: Vec<String>,
    pub supported_regions: Vec<String>,
    pub requires_site_key: bool,
    pub supports_authorized_scope: bool,
    pub max_timeout_ms: u64,
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
            "challenge provider {}: {}",
            self.code,
            self.safe_message.chars().take(256).collect::<String>()
        )
    }
}
impl std::error::Error for ProviderError {}

pub trait ChallengeProvider: Send + Sync {
    fn id(&self) -> &str;
    fn capabilities(&self) -> ProviderCapabilities;
    fn submit(&self, request: &ProviderRequest) -> Result<ProviderResult, ProviderError>;
    fn health(&self) -> bool;
}

pub fn validate_request(request: &ProviderRequest) -> Result<(), ProviderError> {
    if !request.authorized_scope {
        return Err(ProviderError {
            code: "unauthorized_scope".into(),
            safe_message: "provider request is outside an explicitly authorized scope".into(),
        });
    }
    if request.request_id.is_empty()
        || request.request_id.len() > 128
        || request.target_id.is_empty()
        || request.target_id.len() > 128
    {
        return Err(ProviderError {
            code: "invalid_request".into(),
            safe_message: "request identifiers are invalid".into(),
        });
    }
    let url = url::Url::parse(&request.page_url).map_err(|_| ProviderError {
        code: "invalid_url".into(),
        safe_message: "page URL is invalid".into(),
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ProviderError {
            code: "invalid_url".into(),
            safe_message: "page URL must be credential-free http(s)".into(),
        });
    }
    if request.timeout_ms == 0
        || request.timeout_ms > 300_000
        || request.domain.len() > 253
        || request.challenge_kind.len() > 64
    {
        return Err(ProviderError {
            code: "invalid_request".into(),
            safe_message: "provider request exceeds bounds".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unauthorized_scope() {
        let r = ProviderRequest {
            request_id: "r".into(),
            domain: "example.test".into(),
            challenge_kind: "captcha".into(),
            page_url: "https://example.test".into(),
            site_key: None,
            authorized_scope: false,
            timeout_ms: 1000,
            target_id: "t".into(),
        };
        assert_eq!(validate_request(&r).unwrap_err().code, "unauthorized_scope");
    }
}
