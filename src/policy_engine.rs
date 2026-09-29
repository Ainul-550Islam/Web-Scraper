//! Explicit policy evaluation — an ADVISORY, explainable layer over the
//! engine's real safety decisions, not a second enforcement point.
//!
//! The runtime enforces robots/politeness/budgets/breakers inline on the
//! fetch path (that is where it belongs). This module provides a pure,
//! total function over *views* of those subsystems so that operators and
//! tooling can answer "what would the policy do here, and why" — e.g. to
//! annotate an audit line or explain a skip in the console — with the SAME
//! vocabulary the engine uses.
//!
//! Scope, stated plainly:
//! * decisions here are: allow, delay (politeness), deny (robots, limit,
//!   target policy), retry-later (reliability), exhausted (retry cap);
//! * `BlockedByRobots` is TERMINAL for the URL. This layer models no
//!   "workaround" path and the vocabulary has no variant that could express
//!   one: no evasion, no fingerprint spoofing, no CAPTCHA handling, no
//!   ban-dodging, no security-control bypass. A regression test asserts the
//!   enum cannot carry such a reason and that denial output never changes
//!   with proxy/UA inputs (`tests/policy_regression.rs`).
//! * reasons are fixed strings/enum payloads — never URLs, hosts or
//!   user data (bounded labels everywhere).

use serde::Serialize;

/// Robots verdict view. Produced by the caller from `crate::policy`'s parsed
/// robots data (or the engine's RobotsCache); this module never parses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotsVerdict {
    /// robots.txt fetched, parseable, path allowed
    Allowed,
    /// robots.txt says disallow for our token and this path
    Denied,
    /// robots.txt not yet fetched for the host (engine would fetch first)
    NotInspected,
    /// robots.txt could not be fetched ⇒ engine policy is to proceed only
    /// when `respect_robots == false`; advisory here is `Unavailable`
    Unavailable,
}

/// Circuit-breaker view for the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerVerdict {
    Closed,
    HalfOpen,
    Open,
    /// breaker feature not configured — treated as Closed by the engine
    Disabled,
}

/// Why a request may proceed only after waiting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DenyReason {
    BlockedByRobots,
    LimitExceeded { budget: &'static str },
    TargetPolicyRejected,
    ReliabilityBlocked,
    RetryExhausted,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BlockedByRobots => "blocked-by-robots",
            Self::LimitExceeded { .. } => "limit-exceeded",
            Self::TargetPolicyRejected => "target-policy-rejected",
            Self::ReliabilityBlocked => "reliability-blocked",
            Self::RetryExhausted => "retry-exhausted",
        }
    }
}

/// The full decision space. Note what is absent: no variant suggests a way
/// around robots or a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Allow,
    /// polite per-domain spacing says wait this long (capped by the engine)
    Delay {
        wait_ms: u64,
    },
    Deny {
        reason: DenyReason,
    },
    /// transient reliability: try later within retry rules (NOT on robots)
    RetryLater {
        reason: DenyReason,
    },
}

/// Inputs — all views, all cheap; `wait_ms` is what the politeness layer
/// (token bucket + min-interval) would sleep. Budget names are `&'static`
/// (`Budget::new` takes a static name), so the facts borrow nothing.
#[derive(Debug, Clone, Copy)]
pub struct PolicyFacts {
    pub respect_robots: bool,
    pub robots: RobotsVerdict,
    pub target_enabled: bool,
    pub breaker: BreakerVerdict,
    /// engine budget view: which budget (if any) is exhausted
    pub exhausted_budget: Option<&'static str>,
    pub politeness_wait_ms: u64,
    pub attempt: u32,
    pub max_attempts: u32,
}

/// Total, deterministic, side-effect-free policy evaluation. Order mirrors
/// the engine's actual gate order: target policy → robots → hard limits →
/// reliability → politeness timing → allow.
pub fn evaluate(f: &PolicyFacts) -> Decision {
    if !f.target_enabled {
        return Decision::Deny {
            reason: DenyReason::TargetPolicyRejected,
        };
    }
    // robots first, and only when respected; denial is terminal, never
    // retried, never re-routed (see module docs).
    if f.respect_robots {
        match f.robots {
            RobotsVerdict::Denied => {
                return Decision::Deny {
                    reason: DenyReason::BlockedByRobots,
                }
            }
            RobotsVerdict::NotInspected => {
                // The engine fetches robots.txt before deciding; the honest
                // advisory answer is "delay until inspected".
                return Decision::Delay { wait_ms: 0 };
            }
            RobotsVerdict::Unavailable | RobotsVerdict::Allowed => {}
        }
    }
    if let Some(budget) = f.exhausted_budget {
        return Decision::Deny {
            reason: DenyReason::LimitExceeded { budget },
        };
    }
    if f.attempt >= f.max_attempts.max(1) {
        return Decision::Deny {
            reason: DenyReason::RetryExhausted,
        };
    }
    if f.breaker == BreakerVerdict::Open {
        return Decision::RetryLater {
            reason: DenyReason::ReliabilityBlocked,
        };
    }
    if f.politeness_wait_ms > 0 {
        return Decision::Delay {
            wait_ms: f.politeness_wait_ms,
        };
    }
    Decision::Allow
}

/// Machine-readable form for audit annotations (`{"decision":..., "reason"?}`)
/// — stable vocabulary, no free text.
pub fn decision_json(d: &Decision) -> serde_json::Value {
    serde_json::to_value(d).unwrap_or(serde_json::json!({"decision": "allow"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts() -> PolicyFacts {
        PolicyFacts {
            respect_robots: true,
            robots: RobotsVerdict::Allowed,
            target_enabled: true,
            breaker: BreakerVerdict::Disabled,
            exhausted_budget: None,
            politeness_wait_ms: 0,
            attempt: 0,
            max_attempts: 4,
        }
    }

    #[test]
    fn happy_path_allows() {
        assert_eq!(evaluate(&facts()), Decision::Allow);
    }

    #[test]
    fn robots_denial_is_terminal_deny_not_retry() {
        let mut f = facts();
        f.robots = RobotsVerdict::Denied;
        f.breaker = BreakerVerdict::Open; // even with a flaky host…
        f.politeness_wait_ms = 5_000; // …and a long wait pending…
        assert_eq!(
            evaluate(&f),
            Decision::Deny {
                reason: DenyReason::BlockedByRobots
            },
            "robots denial wins and is a Deny (never RetryLater ⇒ nothing to 'retry around it with')"
        );
    }

    #[test]
    fn denial_ignores_evasion_shaped_inputs() {
        // Robots denial must be invariant to everything that could be
        // mistaken for an "alternative egress": breaker state, budget, and
        // respect_robots=false is the ONLY switch and it is operator config.
        let mut a = facts();
        a.robots = RobotsVerdict::Denied;
        let mut b = a;
        b.breaker = BreakerVerdict::HalfOpen;
        b.exhausted_budget = Some("pages");
        b.politeness_wait_ms = 9_999;
        assert_eq!(evaluate(&a), evaluate(&b));
    }

    #[test]
    fn limits_and_ordering() {
        let mut f = facts();
        f.robots = RobotsVerdict::Denied;
        f.respect_robots = false; // operator opted out: robots no longer gates
        f.exhausted_budget = Some("records");
        assert_eq!(
            evaluate(&f),
            Decision::Deny {
                reason: DenyReason::LimitExceeded { budget: "records" }
            }
        );
        f.exhausted_budget = None;
        f.attempt = 4;
        f.max_attempts = 4;
        assert_eq!(
            evaluate(&f),
            Decision::Deny {
                reason: DenyReason::RetryExhausted
            }
        );
        f.attempt = 1;
        f.breaker = BreakerVerdict::Open;
        assert_eq!(
            evaluate(&f),
            Decision::RetryLater {
                reason: DenyReason::ReliabilityBlocked
            }
        );
        f.breaker = BreakerVerdict::Closed;
        f.politeness_wait_ms = 250;
        assert_eq!(evaluate(&f), Decision::Delay { wait_ms: 250 });
    }

    #[test]
    fn uninspected_robots_yields_zero_delay_not_allow() {
        let mut f = facts();
        f.robots = RobotsVerdict::NotInspected;
        assert_eq!(
            decision_json(&evaluate(&f)),
            json!({"decision": "delay", "wait_ms": 0})
        );
    }

    #[test]
    fn vocabulary_carries_no_evasion_semantics() {
        // The decision vocabulary has exactly five shapes and none can
        // express "switch egress / spoof identity / bypass a control".
        // If a variant is ever added, this list must be updated in review.
        let all = [
            Decision::Allow,
            Decision::Delay { wait_ms: 1 },
            Decision::Deny {
                reason: DenyReason::BlockedByRobots,
            },
            Decision::Deny {
                reason: DenyReason::LimitExceeded { budget: "b" },
            },
            Decision::Deny {
                reason: DenyReason::TargetPolicyRejected,
            },
            Decision::Deny {
                reason: DenyReason::ReliabilityBlocked,
            },
            Decision::Deny {
                reason: DenyReason::RetryExhausted,
            },
            Decision::RetryLater {
                reason: DenyReason::ReliabilityBlocked,
            },
        ];
        let strs: Vec<String> = all.iter().map(|d| decision_json(d).to_string()).collect();
        for s in &strs {
            for forbidden in ["proxy", "spoof", "bypass", "evasion", "captcha", "rotate"] {
                assert!(
                    !s.contains(forbidden),
                    "decision JSON leaked `{forbidden}`: {s}"
                );
            }
        }
    }
}
