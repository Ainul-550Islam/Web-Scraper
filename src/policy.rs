//! Per-domain politeness: token bucket + min-interval spacing + a blocking
//! circuit breaker. This is the heart of "compliance-first": pacing is
//! enforced in the engine itself, so every operator inherits it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::metrics::Metrics;

#[derive(Debug)]
struct Bucket {
    tokens: f64,
    capacity: f64,
    refill_per_sec: f64,
    last_refill: Instant,
    last_call: Option<Instant>,
    min_interval: Duration,
    /// Circuit-breaker state, updated by the engine on blocks.
    blocked_until: Option<Instant>,
    consecutive_blocks: u32,
    abandoned: bool,
}

impl Bucket {
    fn next_wait(&mut self, now: Instant) -> Duration {
        if let Some(u) = self.blocked_until {
            if now < u {
                return u - now;
            }
            self.blocked_until = None;
        }
        let dt = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + dt * self.refill_per_sec).min(self.capacity);
        self.last_refill = now;

        let mut wait = Duration::ZERO;
        if self.tokens < 1.0 {
            let need = 1.0 - self.tokens;
            wait = Duration::from_secs_f64(need / self.refill_per_sec.max(0.001));
        }
        if let Some(last) = self.last_call {
            let since = now - last;
            if since < self.min_interval {
                wait = wait.max(self.min_interval - since);
            }
        }
        if wait.is_zero() {
            self.tokens -= 1.0;
            self.last_call = Some(now);
        }
        wait
    }

    /// Consume a permit, sleeping only on the *caller task's* timer, never
    /// holding the shared mutex across the sleep.
    async fn acquire(bucket: &Mutex<Self>, host: &str, metrics: &Metrics) {
        loop {
            let wait = bucket
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .next_wait(Instant::now());
            if wait.is_zero() {
                metrics.count_politeness_wait(host, "permit");
                return;
            }
            metrics.count_politeness_wait(host, "sleep");
            tokio::time::sleep(wait.min(Duration::from_secs(30))).await;
        }
    }

    fn note_block(&mut self, backoff_secs: u64, breaker_count: u32) -> Duration {
        self.consecutive_blocks += 1;
        let exp = self.consecutive_blocks.saturating_sub(1).min(6);
        let secs = backoff_secs.saturating_mul(1u64 << exp).min(3600);
        self.blocked_until = Some(Instant::now() + Duration::from_secs(secs));
        if self.consecutive_blocks >= breaker_count {
            self.abandoned = true;
        }
        Duration::from_secs(secs)
    }
}

pub struct Politeness {
    hosts: Mutex<HashMap<String, Arc<Mutex<Bucket>>>>,
    backoff_secs: u64,
    breaker_count: u32,
}

impl Politeness {
    pub fn new(cfg: &Config) -> Self {
        Self {
            hosts: Mutex::new(HashMap::new()),
            backoff_secs: cfg.block_backoff_secs,
            breaker_count: cfg.block_breaker_count,
        }
    }

    fn bucket(&self, host: &str, cfg: &Config) -> Arc<Mutex<Bucket>> {
        let mut map = self.hosts.lock().unwrap_or_else(|p| p.into_inner());
        map.entry(host.to_string())
            .or_insert_with(|| {
                let p = cfg.policy_for(host);
                Arc::new(Mutex::new(Bucket {
                    tokens: p.burst as f64,
                    capacity: p.burst as f64,
                    refill_per_sec: p.requests_per_sec,
                    last_refill: Instant::now(),
                    last_call: None,
                    min_interval: Duration::from_millis(p.min_interval_ms),
                    blocked_until: None,
                    consecutive_blocks: 0,
                    abandoned: false,
                }))
            })
            .clone()
    }

    /// Tighten a host's min-interval to honor robots.txt `Crawl-delay:`.
    /// Only ever raises the pace the *site asked for*; capped at 60 s so a
    /// hostile/garbage directive can't stall a run (beyond that the site is
    /// saying "don't crawl"; the operator can mirror it in domain_policies).
    pub fn note_crawl_delay(&self, host: &str, cfg: &Config, d: Duration) {
        const CAP: Duration = Duration::from_secs(60);
        if d.is_zero() {
            return;
        }
        if d > CAP {
            tracing::warn!(
                host,
                secs = d.as_secs(),
                "Crawl-delay above 60s; clamping (tune domain_policies to slow further)"
            );
        }
        let d = d.min(CAP);
        let b = self.bucket(host, cfg);
        let mut g = b.lock().unwrap_or_else(|p| p.into_inner());
        if g.min_interval < d {
            tracing::info!(
                host,
                ms = d.as_millis() as u64,
                "honoring robots.txt Crawl-delay (min-interval tightened)"
            );
            g.min_interval = d;
        }
    }

    /// Wait for permission to hit `host` again (token bucket + min-interval).
    pub async fn wait(&self, host: &str, cfg: &Config, metrics: &Metrics) {
        let b = self.bucket(host, cfg);
        Bucket::acquire(&b, host, metrics).await;
    }

    /// Engine calls this after an HTTP 401/403/429/451 from `host`.
    /// `Retry-After`, when present, is honored as a floor.
    pub fn note_block(&self, host: &str, cfg: &Config, retry_after: Option<Duration>) -> Duration {
        let b = self.bucket(host, cfg);
        let mut guard = b.lock().unwrap_or_else(|p| p.into_inner());
        let computed = guard.note_block(self.backoff_secs, self.breaker_count);
        let applied = retry_after.map_or(computed, |ra| {
            ra.max(computed).min(Duration::from_secs(3600))
        });
        guard.blocked_until = Some(Instant::now() + applied);
        let abandoned = guard.abandoned;
        drop(guard);
        if abandoned {
            tracing::warn!(
                host,
                "circuit breaker tripped: abandoning domain for this run"
            );
        } else {
            tracing::warn!(
                host,
                backoff_secs = applied.as_secs(),
                "polite backoff engaged"
            );
        }
        applied
    }

    pub fn note_success(&self, host: &str, cfg: &Config) {
        let b = self.bucket(host, cfg);
        let mut guard = b.lock().unwrap_or_else(|p| p.into_inner());
        guard.consecutive_blocks = 0;
    }

    pub fn is_abandoned(&self, host: &str, cfg: &Config) -> bool {
        let b = self.bucket(host, cfg);
        let guard = b.lock().unwrap_or_else(|p| p.into_inner());
        guard.abandoned
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod crawl_delay_tests {
    use super::*;
    use crate::metrics::Metrics;

    #[tokio::test]
    async fn crawl_delay_only_tightens_and_caps() {
        let mut cfg = Config::default();
        cfg.default_policy.requests_per_sec = 1e9;
        cfg.default_policy.burst = 1000;
        cfg.default_policy.min_interval_ms = 0;
        let pol = Politeness::new(&cfg);
        let m = Metrics::default();

        // Baseline: back-to-back is instant.
        let t0 = Instant::now();
        pol.wait("h", &cfg, &m).await;
        pol.wait("h", &cfg, &m).await;
        assert!(
            t0.elapsed() < Duration::from_millis(50),
            "no pacing before a directive"
        );

        // Site asks for 250ms between requests => honored as a floor.
        pol.note_crawl_delay("h", &cfg, Duration::from_millis(250));
        let t1 = Instant::now();
        pol.wait("h", &cfg, &m).await;
        pol.wait("h", &cfg, &m).await;
        assert!(
            t1.elapsed() >= Duration::from_millis(240),
            "crawl-delay floor enforced: {:?}",
            t1.elapsed()
        );

        // A smaller (or looser-configured) note never *relaxes* pacing,
        // and absurd delays clamp to 60s rather than stalling forever.
        pol.note_crawl_delay("h", &cfg, Duration::from_millis(10));
        let b = pol.bucket("h", &cfg);
        assert_eq!(
            b.lock().unwrap_or_else(|p| p.into_inner()).min_interval,
            Duration::from_millis(250)
        );
        pol.note_crawl_delay("big", &cfg, Duration::from_secs(86_400));
        let b2 = pol.bucket("big", &cfg);
        assert_eq!(
            b2.lock().unwrap_or_else(|p| p.into_inner()).min_interval,
            Duration::from_secs(60)
        );
    }
}
