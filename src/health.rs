//! Typed health domain over the engine's operational surface.
//!
//! The wire types (`ComponentStatus`, `ComponentReport`, `HealthReport`) live
//! in [`crate::api`] and are the single source of truth — this module does
//! NOT redefine them. What lives here is the *decision logic* about when a
//! component earns which status, shared by every producer so the semantics
//! can never fork:
//!
//! * a dependency the deployment did not configure is `Disabled`, never an
//!   error — disabling SQLite must not page anyone;
//! * a dependency that is configured but unreachable right now is
//!   `Unavailable`;
//! * a component that is working but hurting (non-empty quarantine lane, open
//!   breaker for a needed host) is `Degraded`;
//! * aggregation rank/max semantics are `HealthReport::build`'s — including
//!   its rule that an all-Disabled set is Healthy overall.
//!
//! `harvest health --config …` (the offline CLI probe) renders exactly the
//! [`lane_checks`] output below; the parity is pinned by unit tests here and
//! integration tests in `tests/runtime_health.rs`.

use serde_json::{json, Value};

use crate::api::{ComponentReport, ComponentStatus, HealthReport, StorageHealth};
use crate::config::Config;

/// Which family a health-checked dependency belongs to. Used by producers
/// (and the console) to group rows; NOT a new status vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dependency {
    Engine,
    Api,
    Scheduler,
    Workers,
    ProxyPool,
    Redis,
    Postgres,
    Sqlite,
    Storage,
    Export,
    Pipeline,
}

impl Dependency {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Engine => "engine",
            Self::Api => "api",
            Self::Scheduler => "scheduler",
            Self::Workers => "workers",
            Self::ProxyPool => "proxy-pool",
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::Sqlite => "sqlite",
            Self::Storage => "storage",
            Self::Export => "export",
            Self::Pipeline => "pipeline",
        }
    }
}

/// Classify a configured-but-optional dependency honestly:
/// `feature_enabled == false` → Disabled (intentional), `reachable == false`
/// → Unavailable (a problem worth seeing). Never conflate the two.
pub fn classify(feature_enabled: bool, reachable: bool) -> ComponentStatus {
    if !feature_enabled {
        ComponentStatus::Disabled
    } else if !reachable {
        ComponentStatus::Unavailable
    } else {
        ComponentStatus::Healthy
    }
}

/// One lane row for the offline health probe: `{lane, status, detail}` —
/// the exact JSON objects `harvest health` renders as a table.
pub fn lane_checks(cfg: &Config) -> (Vec<Value>, bool) {
    let lanes = serde_json::to_value(StorageHealth::from(cfg)).unwrap_or_else(|_| json!({}));
    let mut checks = Vec::new();
    let mut degraded = false;
    for (k, lane) in lanes.as_object().into_iter().flat_map(|m| m.iter()) {
        let configured = lane["configured"].as_bool().unwrap_or(false);
        let exists = lane["exists"].as_bool().unwrap_or(false);
        let bytes = lane["bytes"].as_u64().unwrap_or(0);
        let (st, why) = if !configured {
            ("disabled", "")
        } else if k == "quarantine" && exists && bytes > 0 {
            degraded = true;
            ("degraded", "quarantine lane has records")
        } else {
            ("healthy", "")
        };
        checks.push(json!({ "lane": k, "status": st, "detail": why }));
    }
    (checks, degraded)
}

/// One breaker row per host the run touched, mapped from the engine's own
/// breaker states (Closed→Healthy, HalfOpen→Degraded, Open→Unavailable:
/// traffic to that host IS not flowing).
pub fn breaker_component(open: usize, half_open: usize, total: usize) -> ComponentReport {
    let status = if open > 0 {
        ComponentStatus::Unavailable
    } else if half_open > 0 {
        ComponentStatus::Degraded
    } else {
        ComponentStatus::Healthy
    };
    ComponentReport {
        name: "breakers",
        status,
        detail: Some(json!({ "hosts": total, "open": open, "half_open": half_open })),
    }
}

/// Aggregate reports with the canonical rules (delegates to
/// [`HealthReport::build`] so there is exactly one aggregation in the tree).
pub fn aggregate(components: Vec<ComponentReport>) -> HealthReport {
    HealthReport::build(components)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn disabled_is_never_an_error_state() {
        assert_eq!(classify(false, false), ComponentStatus::Disabled);
        assert_eq!(classify(false, true), ComponentStatus::Disabled);
        // An all-disabled fleet aggregates to Healthy, not error.
        let rep = aggregate(vec![ComponentReport {
            name: "sqlite",
            status: ComponentStatus::Disabled,
            detail: None,
        }]);
        assert_eq!(rep.overall, ComponentStatus::Healthy);
    }

    #[test]
    fn configured_but_down_is_unavailable() {
        assert_eq!(classify(true, false), ComponentStatus::Unavailable);
        assert_eq!(classify(true, true), ComponentStatus::Healthy);
    }

    fn find<'a>(checks: &'a [Value], lane: &str) -> &'a Value {
        checks
            .iter()
            .find(|c| c["lane"] == json!(lane))
            .unwrap_or_else(|| panic!("row for {lane}"))
    }

    #[test]
    fn quarantine_lane_rules_are_exact() {
        // unconfigured ⇒ disabled (not an error)
        let cfg = Config::default();
        let (checks, degraded) = lane_checks(&cfg);
        assert_eq!(find(&checks, "quarantine")["status"], json!("disabled"));
        assert!(!degraded);

        // configured but file absent ⇒ healthy (nothing quarantined yet)
        let cfg2 = Config {
            targets: vec![quarantine_target("/definitely/missing/q.jsonl")],
            ..Config::default()
        };
        let (checks, degraded) = lane_checks(&cfg2);
        assert_eq!(find(&checks, "quarantine")["status"], json!("healthy"));
        assert!(!degraded);

        // configured WITH records ⇒ degraded + flag set
        let dir = std::env::temp_dir().join(format!("harvest-health-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let q = dir.join("q.jsonl");
        std::fs::write(&q, "{\"x\":1}\n").unwrap();
        let cfg3 = Config {
            targets: vec![quarantine_target(q.to_str().unwrap())],
            ..Config::default()
        };
        let (checks, degraded) = lane_checks(&cfg3);
        let row = find(&checks, "quarantine");
        assert_eq!(row["status"], json!("degraded"));
        assert_eq!(row["detail"], json!("quarantine lane has records"));
        assert!(degraded, "the CLI bails on exactly this flag");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn quarantine_target(path: &str) -> crate::config::TargetCfg {
        let mut t = crate::config::TargetCfg::default();
        t.schema.quarantine_path = Some(std::path::PathBuf::from(path));
        t
    }

    #[test]
    fn breaker_mapping_uses_engine_vocabulary() {
        let c = breaker_component(0, 1, 3);
        assert_eq!(c.status, ComponentStatus::Degraded);
        let c = breaker_component(1, 1, 3);
        assert_eq!(
            c.status,
            ComponentStatus::Unavailable,
            "open dominates half-open"
        );
        let c = breaker_component(0, 0, 3);
        assert_eq!(c.status, ComponentStatus::Healthy);
        assert_eq!(c.detail.unwrap()["hosts"], json!(3));
    }
}
