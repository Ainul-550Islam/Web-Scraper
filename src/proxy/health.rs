//! Bounded endpoint/provider health aggregation.
use super::provider::ProviderHealth;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EndpointHealth {
    pub requests: u64,
    pub failures: u64,
    pub refusals: u64,
    pub latency_ewma_ms: f64,
    pub circuit_open: bool,
    pub last_checked_ms: u64,
}
#[derive(Default)]
pub struct HealthRegistry {
    entries: Mutex<BTreeMap<String, EndpointHealth>>,
}
impl HealthRegistry {
    pub fn report(&self, id: &str, success: bool, refusal: bool, latency_ms: u64) {
        if id.is_empty() || id.len() > 128 {
            return;
        }
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let e = map.entry(id.to_owned()).or_default();
        e.requests = e.requests.saturating_add(1);
        if !success {
            e.failures = e.failures.saturating_add(1);
        }
        if refusal {
            e.refusals = e.refusals.saturating_add(1);
        }
        let sample = latency_ms as f64;
        e.latency_ewma_ms = if e.requests == 1 {
            sample
        } else {
            0.2 * sample + 0.8 * e.latency_ewma_ms
        };
        e.circuit_open = e.requests >= 5 && e.failures.saturating_mul(2) > e.requests;
        e.last_checked_ms = crate::browser::page::now_ms();
    }
    pub fn get(&self, id: &str) -> Option<EndpointHealth> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }
    pub fn snapshot(&self) -> BTreeMap<String, EndpointHealth> {
        self.entries
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
    pub fn update_provider(&self, id: &str, health: &ProviderHealth) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        let e = map.entry(id.into()).or_default();
        e.circuit_open = !health.healthy;
        e.latency_ewma_ms = health.latency_ms.unwrap_or(e.latency_ewma_ms as u64) as f64;
        e.failures = e.failures.max(health.failures);
        e.refusals = e.refusals.max(health.refusals);
        e.last_checked_ms = health.checked_at_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tracks_refusal_separately() {
        let h = HealthRegistry::default();
        h.report("p", false, true, 10);
        let x = h.get("p").unwrap();
        assert_eq!(x.failures, 1);
        assert_eq!(x.refusals, 1);
    }
}
