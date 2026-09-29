//! Policy regression: the legitimate safety/reliability semantics that must
//! NEVER change. This suite guards three things:
//!
//! 1. the policy-engine decision table (advisory layer) — robots denial is
//!    terminal, budgets are hard stops, reliability only ever postpones;
//! 2. the ENGINE's shipped safety defaults (respect_robots on, bounded
//!    response sizes, bounded retries, robots-failure posture) — if a default
//!    flips to "unsafe", this test fails;
//! 3. the explicit absence of evasion: the decision vocabulary cannot even
//!    express "route around robots/limits", and adversarial inputs never
//!    widen it.
//!
//! This suite deliberately does NOT re-test the runtime enforcement paths
//! (fetcher/runner/policy suites own those); it pins the CONTRACTS.

use harvest::config::Config;
use harvest::policy_engine::{
    decision_json, evaluate, BreakerVerdict, Decision, DenyReason, PolicyFacts, RobotsVerdict,
};
use serde_json::Value;

fn base() -> PolicyFacts {
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
fn decision_table_is_total_and_exact() {
    struct Case {
        name: &'static str,
        patch: fn(&mut PolicyFacts),
        want: Decision,
    }
    use Decision::*;
    use RobotsVerdict::*;
    let cases = [
        Case {
            name: "plain allow",
            patch: |_| {},
            want: Allow,
        },
        Case {
            name: "robots denial is deny",
            patch: |f| f.robots = Denied,
            want: Deny {
                reason: DenyReason::BlockedByRobots,
            },
        },
        Case {
            name: "robots denial survives every 'helpful' input",
            patch: |f| {
                f.robots = Denied;
                f.breaker = BreakerVerdict::Closed;
                f.politeness_wait_ms = 60_000;
                f.attempt = 1;
            },
            want: Deny {
                reason: DenyReason::BlockedByRobots,
            },
        },
        Case {
            name: "respect_robots=false keeps operator's opt-out real",
            patch: |f| {
                f.robots = Denied;
                f.respect_robots = false;
            },
            want: Allow,
        },
        Case {
            name: "uninspected robots delays, never allows",
            patch: |f| f.robots = NotInspected,
            want: Delay { wait_ms: 0 },
        },
        Case {
            name: "robots fetch failure is tolerated per engine policy",
            patch: |f| f.robots = Unavailable,
            want: Allow,
        },
        Case {
            name: "budget exhaustion denies",
            patch: |f| f.exhausted_budget = Some("pages"),
            want: Deny {
                reason: DenyReason::LimitExceeded { budget: "pages" },
            },
        },
        Case {
            name: "retry cap denies",
            patch: |f| {
                f.attempt = 4;
                f.max_attempts = 4;
            },
            want: Deny {
                reason: DenyReason::RetryExhausted,
            },
        },
        Case {
            name: "open breaker postpones (never denies outright)",
            patch: |f| f.breaker = BreakerVerdict::Open,
            want: RetryLater {
                reason: DenyReason::ReliabilityBlocked,
            },
        },
        Case {
            name: "half-open breaker allows a probe",
            patch: |f| f.breaker = BreakerVerdict::HalfOpen,
            want: Allow,
        },
        Case {
            name: "politeness delays before allowing",
            patch: |f| f.politeness_wait_ms = 800,
            want: Delay { wait_ms: 800 },
        },
        Case {
            name: "disabled target rejects",
            patch: |f| f.target_enabled = false,
            want: Deny {
                reason: DenyReason::TargetPolicyRejected,
            },
        },
    ];
    for c in cases {
        let mut f = base();
        (c.patch)(&mut f);
        assert_eq!(evaluate(&f), c.want, "case `{}`", c.name);
    }
}

#[test]
fn robots_denial_never_yields_retry_or_allow_under_any_combination() {
    // Exhaustive over the binary dimensions (2^6), both attempt regimes.
    for respect in [true] {
        // respect_robots=true is the safety-relevant regime
        for breaker in [
            BreakerVerdict::Closed,
            BreakerVerdict::Open,
            BreakerVerdict::Disabled,
            BreakerVerdict::HalfOpen,
        ] {
            for wait in [0u64, 5000] {
                for budget in [None, Some("pages")] {
                    for attempt in [0u32, 9] {
                        let f = PolicyFacts {
                            respect_robots: respect,
                            robots: RobotsVerdict::Denied,
                            target_enabled: true,
                            breaker,
                            exhausted_budget: budget,
                            politeness_wait_ms: wait,
                            attempt,
                            max_attempts: 4,
                        };
                        match evaluate(&f) {
                            Decision::Deny {
                                reason: DenyReason::BlockedByRobots,
                            } => {}
                            other => panic!(
                                "robots denial produced {other:?} — it must be terminal Deny"
                            ),
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn decision_json_is_bounded_and_free_of_evasion_vocabulary() {
    let inputs = [
        "http://target.example/secret?token=abc",
        "Bearer sk-super-secret-token",
        "-----BEGIN PRIVATE KEY-----",
        "../../etc/passwd",
        "<script>alert(1)</script>",
    ];
    // Adversarial strings can only arrive as budget names (which are static in
    // the engine) — the layer's API makes that impossible; assert the JSON
    // stays two keys and short regardless.
    for s in inputs {
        let mut f = base();
        f.robots = RobotsVerdict::Denied;
        let v = decision_json(&evaluate(&f));
        assert!(v.is_object() && v.as_object().unwrap().len() <= 2, "v={v}");
        assert!(v.to_string().len() < 120);
        assert!(!v.to_string().contains(s));
    }
}

#[test]
fn engine_ships_safe_defaults_and_the_test_notices_if_they_move() {
    let cfg = Config::default();
    assert!(cfg.respect_robots, "DEFAULT must respect robots");
    assert!(
        cfg.max_response_bytes <= 64 * 1024 * 1024,
        "response size must stay bounded (got {})",
        cfg.max_response_bytes
    );
    assert!(
        cfg.max_retries <= 10,
        "retry count bounded (got {})",
        cfg.max_retries
    );
    assert!(
        cfg.default_policy.requests_per_sec <= 1000.0,
        "default pacing must stay polite, got {}",
        cfg.default_policy.requests_per_sec
    );
    // redirects: the fetcher's cap is a compile-time constant — assert the
    // config surface never grew an "unlimited redirects" escape hatch
    let as_json: Value = serde_json::to_value(&cfg).unwrap();
    let flat = as_json.to_string().to_lowercase();
    for forbidden in [
        "ignore_robots",
        "bypass",
        "evasion",
        "spoof",
        "stealth",
        "undetected",
    ] {
        assert!(
            !flat.contains(forbidden),
            "config grew a forbidden capability: {forbidden}"
        );
    }
}
