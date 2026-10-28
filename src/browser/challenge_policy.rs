//! Operator-controlled challenge decisions. Provider requests and egress
//! changes are explicit; there is no stealth or automatic bypass action.
use super::challenge::{ChallengeKind, ChallengeObservation};
use super::error::{BrowserError, BrowserResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeAction {
    Stop,
    RetryOnce,
    SwitchAuthorizedEgress,
    RequestProvider,
    MarkBlocked,
    ManualReview,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChallengePolicy {
    pub action: ChallengeAction,
    pub allowed_provider_ids: Vec<String>,
    pub allow_egress_switch: bool,
    pub require_audit: bool,
}
impl Default for ChallengePolicy {
    fn default() -> Self {
        Self {
            action: ChallengeAction::Stop,
            allowed_provider_ids: Vec::new(),
            allow_egress_switch: false,
            require_audit: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChallengeDecision {
    pub action: ChallengeAction,
    pub kind: Option<ChallengeKind>,
    pub reason: String,
    pub audit_required: bool,
}
impl ChallengePolicy {
    pub fn validate(&self) -> BrowserResult<()> {
        if self.allowed_provider_ids.len() > 32
            || self
                .allowed_provider_ids
                .iter()
                .any(|s| s.is_empty() || s.len() > 128)
        {
            return Err(BrowserError::InvalidInput("challenge provider allowlist"));
        }
        if self.action == ChallengeAction::SwitchAuthorizedEgress && !self.allow_egress_switch {
            return Err(BrowserError::Policy("egress switching is disabled".into()));
        }
        if self.action == ChallengeAction::RequestProvider && self.allowed_provider_ids.is_empty() {
            return Err(BrowserError::Policy(
                "provider action requires an explicit provider allowlist".into(),
            ));
        }
        Ok(())
    }
    pub fn decide(&self, observation: &ChallengeObservation) -> BrowserResult<ChallengeDecision> {
        self.validate()?;
        if !observation.detected {
            return Ok(ChallengeDecision {
                action: ChallengeAction::Stop,
                kind: None,
                reason: "no challenge detected".into(),
                audit_required: false,
            });
        }
        let action = match self.action {
            ChallengeAction::SwitchAuthorizedEgress if !self.allow_egress_switch => {
                return Err(BrowserError::Policy(
                    "authorized egress switching is not enabled".into(),
                ))
            }
            ChallengeAction::RequestProvider if self.allowed_provider_ids.is_empty() => {
                return Err(BrowserError::Policy(
                    "no authorized challenge provider configured".into(),
                ))
            }
            a => a,
        };
        Ok(ChallengeDecision {
            action,
            kind: observation.kind,
            reason: "observable challenge; operator policy applied without evasion".into(),
            audit_required: self.require_audit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::challenge::ChallengeObservation;
    #[test]
    fn default_policy_stops_and_audits() {
        let p = ChallengePolicy::default();
        let o = ChallengeObservation {
            detected: true,
            kind: Some(ChallengeKind::Captcha),
            confidence: 1.0,
            evidence: vec!["captcha".into()],
            url: "https://site.test".into(),
            title: None,
        };
        let d = p.decide(&o).unwrap();
        assert_eq!(d.action, ChallengeAction::Stop);
        assert!(d.audit_required);
    }
}
