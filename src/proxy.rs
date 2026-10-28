//! Upstream proxy pool with health scoring.
//!
//! Provider, lease, health, and browser capabilities are additive modules;
//! the legacy HTTP pool below retains its original selection semantics.
//!
//! Honest scope note: rotation here exists for **reliability** (a flaky
//! provider gateway gets drained automatically) and **geography** (regional
//! result sets). It is deliberately *not* wired to block handling: when a
//! server refuses us, the politeness layer backs off that domain; the pool
//! never "hides" a refusal.

pub mod browser;
pub mod credentials;
pub mod health;
pub mod lease;
pub mod policy;
pub mod provider;
pub mod registry;

use std::sync::Mutex;

use crate::config::ProxyCfg;

#[derive(Debug, Clone)]
pub struct ProxyEntry {
    pub id: String,
    pub url: Option<String>,
    pub region: Option<String>,
    pub latency_ewma_ms: f64,
    pub success_ewma: f64,
    pub requests: u64,
    pub failures: u64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ProxySnapshot {
    pub id: String,
    pub region: Option<String>,
    pub latency_ewma_ms: f64,
    pub success_ewma: f64,
    pub requests: u64,
    pub failures: u64,
    /// Requests currently in flight through this egress (0 when no
    /// in-flight cap is configured and nobody is holding).
    pub inflight: u64,
}

pub struct ProxyPool {
    entries: Mutex<Vec<ProxyEntry>>,
    alpha: f64,
    /// Simultaneous-request holds per entry (parallel to `entries`; kept
    /// out of `ProxyEntry` so the snapshot type stays copy-able).
    inflight: Mutex<Vec<u64>>,
    /// 0 = unlimited (the default: legacy behavior byte-for-byte).
    cap: std::sync::atomic::AtomicU64,
    /// Selections refused because every candidate sat at the cap.
    denied: std::sync::atomic::AtomicU64,
}

/// RAII in-flight hold; releasing is automatic even on panic paths.
pub struct ProxyHold {
    pool: std::sync::Arc<ProxyPool>,
    idx: usize,
}

impl Drop for ProxyHold {
    fn drop(&mut self) {
        self.pool.release_hold(self.idx);
    }
}

impl ProxyPool {
    pub fn new(proxies: &[ProxyCfg]) -> Self {
        let mut entries: Vec<ProxyEntry> = if proxies.is_empty() {
            vec![ProxyEntry {
                id: "direct".into(),
                url: None,
                region: None,
                latency_ewma_ms: 50.0,
                success_ewma: 1.0,
                requests: 0,
                failures: 0,
            }]
        } else {
            proxies
                .iter()
                .map(|p| ProxyEntry {
                    id: p.id.clone(),
                    url: p.url.clone(),
                    region: p.region.clone(),
                    latency_ewma_ms: 50.0,
                    success_ewma: 1.0,
                    requests: 0,
                    failures: 0,
                })
                .collect()
        };
        entries.sort_by(|a, b| a.id.cmp(&b.id)); // deterministic layout
        let n = if proxies.is_empty() { 1 } else { proxies.len() };
        Self {
            entries: Mutex::new(entries),
            alpha: 0.2,
            inflight: Mutex::new(vec![0; n]),
            cap: std::sync::atomic::AtomicU64::new(0),
            denied: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Configure the per-proxy in-flight cap (`max_inflight_per_proxy`;
    /// 0 = unlimited). Callers that hold an `Arc` get selection steering
    /// for free; uncapped pools behave exactly as before.
    pub fn set_inflight_cap(&self, cap: u64) {
        self.cap.store(cap, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn inflight_cap(&self) -> u64 {
        self.cap.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn denied(&self) -> u64 {
        self.denied.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Take a hold on `idx` unless that egress sits at its cap. Returns
    /// `None` when the cap is full — the caller treats it like a transient
    /// transport refusal (retryable, backed off), never a silent queue.
    pub fn try_hold(self: &std::sync::Arc<Self>, idx: usize) -> Option<ProxyHold> {
        let cap = self.inflight_cap();
        let mut g = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
        let slot = g.get_mut(idx)?;
        if cap > 0 && *slot >= cap {
            drop(g);
            self.denied
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        *slot += 1;
        Some(ProxyHold {
            pool: self.clone(),
            idx,
        })
    }

    fn release_hold(&self, idx: usize) {
        let mut g = self.inflight.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get_mut(idx) {
            *s = s.saturating_sub(1);
        }
    }

    /// Live in-flight count per entry (diagnostics/tests).
    pub fn inflight(&self) -> Vec<u64> {
        self.inflight
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn weight(e: &ProxyEntry) -> f64 {
        // Reliability-weighted; a proxy with 0.5 success and high latency is
        // exponentially less likely to be chosen.
        let rel = e.success_ewma.max(0.01);
        (rel * rel) / (1.0 + e.latency_ewma_ms / 500.0)
    }

    /// Deterministic weighted pick for a given salt (nanos counter etc).
    pub fn pick(&self, salt: u64) -> usize {
        let g = self.entries.lock().unwrap();
        let weights: Vec<f64> = g.iter().map(Self::weight).collect();
        let total: f64 = weights.iter().sum();
        if total <= 0.0 {
            return 0;
        }
        // scale to fixed-point integers to avoid float modulo quirks
        const SCALE: f64 = 1e6;
        let ints: Vec<u64> = weights
            .iter()
            .map(|w| (w * SCALE).max(1.0) as u64)
            .collect();
        let total_i: u128 = ints.iter().sum::<u64>() as u128;
        let mut target = (salt as u128 % total_i) as u64;
        for (i, w) in ints.iter().enumerate() {
            if target < *w {
                return i;
            }
            target -= *w;
        }
        0
    }

    pub fn proxy_url(&self, idx: usize) -> Option<Option<String>> {
        let g = self.entries.lock().unwrap();
        g.get(idx).map(|e| e.url.clone())
    }

    pub fn id(&self, idx: usize) -> String {
        let g = self.entries.lock().unwrap();
        g.get(idx)
            .map(|e| e.id.clone())
            .unwrap_or_else(|| "direct".into())
    }

    pub fn report(&self, idx: usize, ok: bool, latency_ms: u64) {
        let mut g = self.entries.lock().unwrap();
        let a = self.alpha;
        if let Some(e) = g.get_mut(idx) {
            e.requests += 1;
            if !ok {
                e.failures += 1;
            }
            e.latency_ewma_ms = (1.0 - a) * e.latency_ewma_ms + a * latency_ms as f64;
            e.success_ewma = (1.0 - a) * e.success_ewma + a * if ok { 1.0 } else { 0.0 };
        }
    }

    pub fn snapshot(&self) -> Vec<ProxySnapshot> {
        let inflight = self
            .inflight
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let g = self.entries.lock().unwrap();
        g.iter()
            .enumerate()
            .map(|(i, e)| ProxySnapshot {
                id: e.id.clone(),
                region: e.region.clone(),
                latency_ewma_ms: e.latency_ewma_ms,
                success_ewma: e.success_ewma,
                requests: e.requests,
                failures: e.failures,
                inflight: inflight.get(i).copied().unwrap_or(0),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
