use super::budget::{BudgetLimit, ChallengeBudget};
use super::provider::*;
use super::registry::ChallengeRegistry;
use super::unblock::*;
use std::sync::Arc;

struct TestProvider;
impl ChallengeProvider for TestProvider {
    fn id(&self) -> &str {
        "test-provider"
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            challenge_kinds: vec!["captcha".into()],
            supports_authorized_scope: true,
            max_timeout_ms: 10_000,
            ..Default::default()
        }
    }
    fn submit(&self, r: &ProviderRequest) -> Result<ProviderResult, ProviderError> {
        validate_request(r)?;
        Ok(ProviderResult {
            request_id: r.request_id.clone(),
            status: ProviderStatus::Accepted,
            cost_microusd: 7,
            elapsed_ms: 12,
            reference: Some("opaque-ref".into()),
            safe_message: None,
        })
    }
    fn health(&self) -> bool {
        true
    }
}

#[test]
fn provider_selection_budget_and_audit_are_joined_by_request_id() {
    let registry = ChallengeRegistry::default();
    registry
        .register(Arc::new(TestProvider), vec!["example.test".into()])
        .unwrap();
    let provider = registry
        .select("www.example.test", "captcha", &["test-provider".into()])
        .unwrap();
    let budget = ChallengeBudget::new(BudgetLimit {
        max_requests: 1,
        max_cost_microusd: 10,
    })
    .unwrap();
    budget.reserve("target-a", provider.id()).unwrap();
    let payload = ProviderRequest {
        request_id: "req-1".into(),
        domain: "www.example.test".into(),
        challenge_kind: "captcha".into(),
        page_url: "https://www.example.test/".into(),
        site_key: Some("public-site-key".into()),
        authorized_scope: true,
        timeout_ms: 5000,
        target_id: "target-a".into(),
    };
    let authorization = UnblockAuthorization {
        domain: payload.domain.clone(),
        target_id: payload.target_id.clone(),
        policy_reference: "approval-42".into(),
        approved: true,
        expires_at_ms: u64::MAX,
    };
    let (result, audit) = request(provider.as_ref(), payload, &authorization).unwrap();
    budget
        .apply_result("target-a", provider.id(), &result)
        .unwrap();
    assert_eq!(result.request_id, audit.request_id);
    assert_eq!(audit.policy_reference, "approval-42");
    assert!(budget.reserve("target-a", provider.id()).is_err());
}

#[test]
fn provider_request_does_not_accept_plain_unauthorized_scope() {
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
    assert!(validate_request(&r).is_err());
}
