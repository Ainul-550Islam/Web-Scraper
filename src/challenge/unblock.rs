//! Correlated authorized unblock-provider requests. The integration returns
//! provider metadata only; it never replays a request or overrides a refusal.
use super::provider::{
    validate_request, ChallengeProvider, ProviderError, ProviderRequest, ProviderResult,
    ProviderStatus,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnblockAuthorization {
    pub domain: String,
    pub target_id: String,
    pub policy_reference: String,
    pub approved: bool,
    pub expires_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UnblockAudit {
    pub request_id: String,
    pub provider_id: String,
    pub domain: String,
    pub target_id: String,
    pub status: ProviderStatus,
    pub cost_microusd: u64,
    pub elapsed_ms: u64,
    pub policy_reference: String,
    pub created_at_ms: u64,
}

pub fn request(
    provider: &dyn ChallengeProvider,
    request: ProviderRequest,
    authorization: &UnblockAuthorization,
) -> Result<(ProviderResult, UnblockAudit), ProviderError> {
    if !authorization.approved
        || authorization.domain != request.domain
        || authorization.target_id != request.target_id
        || authorization.expires_at_ms <= crate::browser::page::now_ms()
        || authorization.policy_reference.is_empty()
    {
        return Err(ProviderError {
            code: "authorization_denied".into(),
            safe_message: "unblock request lacks a current matching authorization".into(),
        });
    }
    validate_request(&request)?;
    let caps = provider.capabilities();
    if !caps.supports_authorized_scope
        || !caps
            .challenge_kinds
            .iter()
            .any(|k| k == &request.challenge_kind)
    {
        return Err(ProviderError {
            code: "unsupported_capability".into(),
            safe_message: "provider does not support this authorized challenge type".into(),
        });
    }
    let result = provider.submit(&request)?;
    let audit = UnblockAudit {
        request_id: request.request_id.clone(),
        provider_id: provider.id().to_owned(),
        domain: request.domain,
        target_id: request.target_id,
        status: result.status,
        cost_microusd: result.cost_microusd,
        elapsed_ms: result.elapsed_ms,
        policy_reference: authorization.policy_reference.clone(),
        created_at_ms: crate::browser::page::now_ms(),
    };
    Ok((result, audit))
}

#[cfg(test)]
mod tests {
    use super::super::provider::ProviderCapabilities;
    use super::*;
    struct P;
    impl ChallengeProvider for P {
        fn id(&self) -> &str {
            "p"
        }
        fn capabilities(&self) -> ProviderCapabilities {
            ProviderCapabilities {
                challenge_kinds: vec!["captcha".into()],
                supports_authorized_scope: true,
                ..Default::default()
            }
        }
        fn submit(&self, r: &ProviderRequest) -> Result<ProviderResult, ProviderError> {
            Ok(ProviderResult {
                request_id: r.request_id.clone(),
                status: ProviderStatus::Accepted,
                cost_microusd: 1,
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
    fn mismatched_authorization_is_refused() {
        let req = ProviderRequest {
            request_id: "r".into(),
            domain: "example.test".into(),
            challenge_kind: "captcha".into(),
            page_url: "https://example.test".into(),
            site_key: None,
            authorized_scope: true,
            timeout_ms: 5000,
            target_id: "t".into(),
        };
        let auth = UnblockAuthorization {
            domain: "other.test".into(),
            target_id: "t".into(),
            policy_reference: "approval".into(),
            approved: true,
            expires_at_ms: u64::MAX,
        };
        assert!(request(&P, req, &auth).is_err());
    }
}
