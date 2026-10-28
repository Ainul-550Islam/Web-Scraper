//! Browser failure-aware retry decisions. Deterministic policy refusals and
//! challenges are not retried blindly.
use super::error::BrowserError;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BrowserFailure {
    NavigationTransient,
    BrowserCrash,
    ProviderFailure,
    SessionExpired,
    TargetServerTransient,
    PolicyRefusal,
    Challenge,
    InvalidSelector,
    JavaScriptError,
    ContentLimit,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrowserRetryPolicy {
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub deterministic: bool,
}
impl Default for BrowserRetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            base_delay_ms: 500,
            max_delay_ms: 15_000,
            deterministic: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RetryDecision {
    pub retry: bool,
    pub attempt: u32,
    pub delay_ms: u64,
    pub reason: String,
}
impl BrowserRetryPolicy {
    pub fn decide(&self, failure: BrowserFailure, attempt: u32) -> RetryDecision {
        let retryable = matches!(
            failure,
            BrowserFailure::NavigationTransient
                | BrowserFailure::BrowserCrash
                | BrowserFailure::ProviderFailure
                | BrowserFailure::SessionExpired
                | BrowserFailure::TargetServerTransient
        );
        if !retryable || attempt >= self.max_attempts {
            return RetryDecision {
                retry: false,
                attempt,
                delay_ms: 0,
                reason: format!("{failure:?} is not retryable or the attempt budget is exhausted"),
            };
        }
        let exp = self
            .base_delay_ms
            .saturating_mul(
                1u64.checked_shl(attempt.saturating_sub(1).min(31))
                    .unwrap_or(u64::MAX),
            )
            .min(self.max_delay_ms);
        let jitter = if self.deterministic { 0 } else { exp / 10 };
        RetryDecision {
            retry: true,
            attempt: attempt + 1,
            delay_ms: exp.saturating_add(jitter).min(self.max_delay_ms),
            reason: format!("retryable {failure:?}"),
        }
    }
    pub fn classify(error: &BrowserError) -> BrowserFailure {
        match error {
            BrowserError::Timeout {
                operation: "navigation",
            }
            | BrowserError::Connection => BrowserFailure::NavigationTransient,
            BrowserError::Crashed => BrowserFailure::BrowserCrash,
            BrowserError::Shutdown => BrowserFailure::SessionExpired,
            BrowserError::Policy(_) => BrowserFailure::PolicyRefusal,
            BrowserError::NotFound("element") | BrowserError::InvalidInput("CSS selector") => {
                BrowserFailure::InvalidSelector
            }
            BrowserError::Javascript(_) => BrowserFailure::JavaScriptError,
            BrowserError::ContentLimit { .. } => BrowserFailure::ContentLimit,
            _ => BrowserFailure::TargetServerTransient,
        }
    }
    pub fn delay(&self, d: &RetryDecision) -> Duration {
        Duration::from_millis(d.delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn does_not_retry_policy_refusal() {
        let p = BrowserRetryPolicy::default();
        assert!(!p.decide(BrowserFailure::PolicyRefusal, 1).retry);
    }
    #[test]
    fn retries_transient_with_cap() {
        let p = BrowserRetryPolicy {
            max_attempts: 3,
            base_delay_ms: 10,
            max_delay_ms: 20,
            deterministic: true,
        };
        let d = p.decide(BrowserFailure::NavigationTransient, 3);
        assert!(!d.retry);
        assert!(d.delay_ms <= 20);
    }
}
