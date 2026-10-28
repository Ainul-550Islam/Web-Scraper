//! Thread-safe, configuration-driven registry of proxy providers.
use super::provider::{
    ProviderCapabilities, ProviderError, ProviderHealth, ProxyEndpoint, ProxyProvider, ProxyRequest,
};
use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

pub struct ProxyRegistry {
    providers: RwLock<BTreeMap<String, Arc<dyn ProxyProvider>>>,
}
impl Default for ProxyRegistry {
    fn default() -> Self {
        Self {
            providers: RwLock::new(BTreeMap::new()),
        }
    }
}
impl ProxyRegistry {
    pub fn register(&self, provider: Arc<dyn ProxyProvider>) -> Result<(), String> {
        let id = provider.id().to_owned();
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("invalid provider id".into());
        }
        let mut map = self.providers.write().unwrap_or_else(|p| p.into_inner());
        if map.contains_key(&id) {
            return Err(format!("proxy provider `{id}` is already registered"));
        }
        map.insert(id, provider);
        Ok(())
    }
    pub fn get(&self, id: &str) -> Option<Arc<dyn ProxyProvider>> {
        self.providers
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(id)
            .cloned()
    }
    pub fn ids(&self) -> Vec<String> {
        self.providers
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .keys()
            .cloned()
            .collect()
    }
    pub fn capabilities(&self) -> BTreeMap<String, ProviderCapabilities> {
        self.providers
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .map(|(id, p)| (id.clone(), p.capabilities()))
            .collect()
    }
}

/// Static provider useful for configured fixed proxy endpoints and tests.
pub struct StaticProxyProvider {
    id: String,
    endpoints: Vec<ProxyEndpoint>,
    health: std::sync::Mutex<ProviderHealth>,
    cursor: std::sync::atomic::AtomicUsize,
}
impl StaticProxyProvider {
    pub fn new(id: impl Into<String>, endpoints: Vec<ProxyEndpoint>) -> Result<Self, String> {
        let id = id.into();
        if id.is_empty() || endpoints.is_empty() || endpoints.len() > 256 {
            return Err("static proxy provider requires an id and 1..=256 endpoints".into());
        }
        Ok(Self {
            id,
            endpoints,
            health: std::sync::Mutex::new(ProviderHealth {
                healthy: true,
                latency_ms: None,
                failures: 0,
                refusals: 0,
                checked_at_ms: crate::browser::page::now_ms(),
                message: None,
            }),
            cursor: std::sync::atomic::AtomicUsize::new(0),
        })
    }
}
impl ProxyProvider for StaticProxyProvider {
    fn id(&self) -> &str {
        &self.id
    }
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            regions: self
                .endpoints
                .iter()
                .filter_map(|e| e.region.clone())
                .collect(),
            session_affinity: self.endpoints.iter().all(|e| e.supports_session_affinity),
            authenticated_endpoints: self
                .endpoints
                .iter()
                .any(|e| e.username_secret_ref.is_some() || e.password_secret_ref.is_some()),
            leases: false,
            max_concurrency: None,
        }
    }
    fn acquire(&self, request: &ProxyRequest) -> Result<ProxyEndpoint, ProviderError> {
        let matching: Vec<_> = self
            .endpoints
            .iter()
            .filter(|e| {
                request
                    .preferred_region
                    .as_ref()
                    .is_none_or(|r| e.region.as_ref() == Some(r))
            })
            .collect();
        if matching.is_empty() {
            return Err(ProviderError {
                code: "no_endpoint".into(),
                safe_message: "no endpoint matches the requested region".into(),
            });
        }
        let index = self
            .cursor
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % matching.len();
        Ok(matching[index].clone())
    }
    fn report(&self, _endpoint_id: &str, success: bool, refusal: bool, latency_ms: u64) {
        let mut h = self.health.lock().unwrap_or_else(|p| p.into_inner());
        if !success {
            h.failures = h.failures.saturating_add(1);
        }
        if refusal {
            h.refusals = h.refusals.saturating_add(1);
        }
        h.latency_ms = Some(
            h.latency_ms
                .map(|old| ((old as f64 * 0.8) + (latency_ms as f64 * 0.2)) as u64)
                .unwrap_or(latency_ms),
        );
        h.healthy = h.failures < 10;
        h.checked_at_ms = crate::browser::page::now_ms();
    }
    fn health(&self) -> ProviderHealth {
        self.health
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_registry_rejects_duplicate() {
        let r = ProxyRegistry::default();
        let p = Arc::new(
            StaticProxyProvider::new(
                "p",
                vec![ProxyEndpoint {
                    id: "e".into(),
                    url: "http://proxy:80".into(),
                    region: None,
                    username_secret_ref: None,
                    password_secret_ref: None,
                    supports_session_affinity: false,
                    supports_region_selection: false,
                }],
            )
            .unwrap(),
        );
        assert!(r.register(p.clone()).is_ok());
        assert!(r.register(p).is_err());
    }
}
