//! Bounded, expiring proxy lease book with deterministic release semantics.
use super::provider::ProxyEndpoint;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct LeaseSnapshot {
    pub lease_id: String,
    pub endpoint_id: String,
    pub target_id: String,
    pub active_for_ms: u64,
}
struct Lease {
    endpoint: ProxyEndpoint,
    target: String,
    started: Instant,
    expires: Instant,
}
pub struct LeaseBook {
    leases: Mutex<HashMap<String, Lease>>,
    max_active: usize,
    ttl: Duration,
}
pub struct LeaseGuard {
    book: Arc<LeaseBook>,
    id: String,
    endpoint: ProxyEndpoint,
    released: bool,
}
impl LeaseBook {
    pub fn new(max_active: usize, ttl: Duration) -> Result<Arc<Self>, &'static str> {
        if max_active == 0
            || max_active > 10_000
            || ttl.is_zero()
            || ttl > Duration::from_secs(86_400)
        {
            return Err("lease bounds must be 1..=10000 and TTL <= 24h");
        }
        Ok(Arc::new(Self {
            leases: Mutex::new(HashMap::new()),
            max_active,
            ttl,
        }))
    }
    pub fn acquire(
        self: &Arc<Self>,
        id: String,
        target: String,
        endpoint: ProxyEndpoint,
    ) -> Result<LeaseGuard, &'static str> {
        if id.is_empty() || id.len() > 128 || target.is_empty() || target.len() > 128 {
            return Err("invalid proxy lease id or target");
        }
        let mut leases = self.leases.lock().unwrap_or_else(|p| p.into_inner());
        leases.retain(|_, l| l.expires > Instant::now());
        if leases.len() >= self.max_active {
            return Err("proxy lease capacity exhausted");
        }
        if leases.contains_key(&id) {
            return Err("duplicate proxy lease id");
        }
        let now = Instant::now();
        leases.insert(
            id.clone(),
            Lease {
                endpoint: endpoint.clone(),
                target,
                started: now,
                expires: now + self.ttl,
            },
        );
        Ok(LeaseGuard {
            book: self.clone(),
            id,
            endpoint,
            released: false,
        })
    }
    pub fn snapshots(&self) -> Vec<LeaseSnapshot> {
        let now = Instant::now();
        let mut leases = self.leases.lock().unwrap_or_else(|p| p.into_inner());
        leases.retain(|_, l| l.expires > now);
        leases
            .iter()
            .map(|(id, l)| LeaseSnapshot {
                lease_id: id.clone(),
                endpoint_id: l.endpoint.id.clone(),
                target_id: l.target.clone(),
                active_for_ms: l.started.elapsed().as_millis() as u64,
            })
            .collect()
    }
    fn release(&self, id: &str) -> bool {
        self.leases
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
            .is_some()
    }
}
impl LeaseGuard {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn endpoint(&self) -> &ProxyEndpoint {
        &self.endpoint
    }
    pub fn release(mut self) -> bool {
        self.released = true;
        self.book.release(&self.id)
    }
}
impl Drop for LeaseGuard {
    fn drop(&mut self) {
        if !self.released {
            self.book.release(&self.id);
            self.released = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::provider::ProxyEndpoint;
    fn ep() -> ProxyEndpoint {
        ProxyEndpoint {
            id: "e".into(),
            url: "http://proxy.test:80".into(),
            region: None,
            username_secret_ref: None,
            password_secret_ref: None,
            supports_session_affinity: false,
            supports_region_selection: false,
        }
    }
    #[test]
    fn lease_is_released_on_drop() {
        let b = LeaseBook::new(1, Duration::from_secs(10)).unwrap();
        {
            let _g = b.acquire("l".into(), "t".into(), ep()).unwrap();
            assert_eq!(b.snapshots().len(), 1);
        }
        assert!(b.snapshots().is_empty());
    }
}
