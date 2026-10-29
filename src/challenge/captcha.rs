//! CAPTCHA-specific provider adapter contract; no provider credentials or
//! vendor API are built in.
use super::provider::{ChallengeProvider, ProviderError, ProviderRequest};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptchaRequest {
    pub request_id: String,
    pub provider_id: String,
    pub domain: String,
    pub page_url: String,
    pub site_key: String,
    pub authorized: bool,
    pub timeout_ms: u64,
    pub target_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptchaSubmission {
    pub request_id: String,
    pub provider_id: String,
    pub accepted: bool,
    pub cost_microusd: u64,
    pub elapsed_ms: u64,
    pub reference: Option<String>,
}

pub fn submit(
    provider: &dyn ChallengeProvider,
    request: &CaptchaRequest,
) -> Result<CaptchaSubmission, ProviderError> {
    if !request.authorized {
        return Err(ProviderError {
            code: "unauthorized_scope".into(),
            safe_message: "CAPTCHA provider call requires explicit authorization".into(),
        });
    }
    if request.site_key.is_empty()
        || request.site_key.len() > 1024
        || request.site_key.chars().any(char::is_control)
    {
        return Err(ProviderError {
            code: "invalid_site_key".into(),
            safe_message: "CAPTCHA site key is invalid".into(),
        });
    }
    let caps = provider.capabilities();
    if !caps.challenge_kinds.iter().any(|k| k == "captcha") || !caps.supports_authorized_scope {
        return Err(ProviderError {
            code: "unsupported_capability".into(),
            safe_message: "provider does not declare authorized CAPTCHA support".into(),
        });
    }
    if caps.max_timeout_ms > 0 && request.timeout_ms > caps.max_timeout_ms {
        return Err(ProviderError {
            code: "timeout_exceeds_capability".into(),
            safe_message: "requested timeout exceeds provider capability".into(),
        });
    }
    let generic = ProviderRequest {
        request_id: request.request_id.clone(),
        domain: request.domain.clone(),
        challenge_kind: "captcha".into(),
        page_url: request.page_url.clone(),
        site_key: Some(request.site_key.clone()),
        authorized_scope: true,
        timeout_ms: request.timeout_ms,
        target_id: request.target_id.clone(),
    };
    let result = provider.submit(&generic)?;
    Ok(CaptchaSubmission {
        request_id: result.request_id,
        provider_id: request.provider_id.clone(),
        accepted: matches!(
            result.status,
            super::provider::ProviderStatus::Accepted | super::provider::ProviderStatus::Completed
        ),
        cost_microusd: result.cost_microusd,
        elapsed_ms: result.elapsed_ms,
        reference: result.reference,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::challenge::provider::{self, ProviderResult};
    struct Noop;
    impl ChallengeProvider for Noop {
        fn id(&self) -> &str {
            "noop"
        }
        fn capabilities(&self) -> provider::ProviderCapabilities {
            provider::ProviderCapabilities {
                challenge_kinds: vec!["captcha".into()],
                supports_authorized_scope: true,
                max_timeout_ms: 30_000,
                ..Default::default()
            }
        }
        fn submit(&self, r: &ProviderRequest) -> Result<ProviderResult, ProviderError> {
            Ok(ProviderResult {
                request_id: r.request_id.clone(),
                status: provider::ProviderStatus::Accepted,
                cost_microusd: 0,
                elapsed_ms: 1,
                reference: None,
                safe_message: None,
            })
        }
        fn health(&self) -> bool {
            true
        }
    }
    #[test]
    fn authorized_submission_requires_provider_capability() {
        let req = CaptchaRequest {
            request_id: "r".into(),
            provider_id: "noop".into(),
            domain: "example.test".into(),
            page_url: "https://example.test".into(),
            site_key: "public-key".into(),
            authorized: true,
            timeout_ms: 1000,
            target_id: "t".into(),
        };
        assert!(submit(&Noop, &req).unwrap().accepted);
    }
}
