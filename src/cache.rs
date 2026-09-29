//! Bounded TTL cache — the one cache abstraction in the tree.
//!
//! Design rules (all enforced by types or tests):
//! * capacity is a hard bound — `insert` evicts before it grows, so memory
//!   can never be O(distinct keys);
//! * every entry carries an absolute expiry from an injectable [`Clock`], so
//!   tests advance time deterministically instead of sleeping;
//! * `get` of an expired key is a miss AND removes the entry (no stale reads);
//! * hit/miss/eviction/expiry counters exist so a caller can prove the cache
//!   is working — and nobody can quietly swap in an unbounded map, because
//!   all access goes through this type;
//! * policy decisions are NEVER cached here: this module knows nothing about
//!   robots, budgets or breakers by construction.
//!
//! Thread-safe: one mutex around a small, never-slow critical section
//! (HashMap + VecDeque touch). Lock poisoning is recovered (the inner state
//! is always consistent by invariant) exactly like the rest of the engine.

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Time source. Swap for [`ManualClock`] in tests.
pub trait Clock: Send + Sync {
    fn now_ms(&self) -> u64;
}

pub struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

/// Test clock: starts at 0, advanced explicitly.
pub struct ManualClock(Mutex<u64>);
impl ManualClock {
    pub fn new() -> Self {
        Self(Mutex::new(0))
    }
    pub fn advance(&self, d: Duration) {
        let mut g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        *g += d.as_millis() as u64;
    }
}
impl Default for ManualClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub expired: u64,
    pub evictions: u64,
    pub len: usize,
    pub capacity: usize,
}

impl CacheStats {
    /// 0.0..=1.0; 0.0 when nothing has been looked up (never NaN, never
    /// "1.0 on an idle cache" — an honest zero).
    pub fn hit_ratio(&self) -> f64 {
        let t = self.hits + self.misses;
        if t == 0 {
            0.0
        } else {
            self.hits as f64 / t as f64
        }
    }
}

struct Entry<V> {
    value: V,
    expires_at_ms: Option<u64>,
}

/// `capacity == 0` is legal and means "always miss" (kill-switch deployments)
/// — still bounded, still counted.
pub struct BoundedCache<K, V> {
    inner: Mutex<CacheInner<K, V>>,
    capacity: usize,
    ttl_ms: Option<u64>,
    clock: Arc<dyn Clock>,
}

struct CacheInner<K, V> {
    map: HashMap<K, Entry<V>>,
    /// oldest-first LRU order; front = next eviction victim
    order: VecDeque<K>,
    stats: StatsAccum,
}

#[derive(Default)]
struct StatsAccum {
    hits: u64,
    misses: u64,
    expired: u64,
    evictions: u64,
}

impl<K: Clone + Eq + Hash, V: Clone> BoundedCache<K, V> {
    pub fn new(capacity: usize, ttl: Option<Duration>, clock: Arc<dyn Clock>) -> Self {
        Self {
            inner: Mutex::new(CacheInner {
                map: HashMap::new(),
                order: VecDeque::new(),
                stats: StatsAccum::default(),
            }),
            capacity,
            ttl_ms: ttl.map(|d| d.as_millis().min(u64::MAX as u128) as u64),
            clock,
        }
    }

    pub fn get(&self, key: &K) -> Option<V> {
        let now = self.clock.now_ms();
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let found = g.map.get(key).map(|e| (e.value.clone(), e.expires_at_ms));
        match found {
            Some((value, expires)) if !expires.is_some_and(|x| x <= now) => {
                g.stats.hits += 1;
                g.order.retain(|k| k != key);
                g.order.push_back(key.clone());
                Some(value)
            }
            Some(_) => {
                g.map.remove(key);
                g.order.retain(|k| k != key);
                g.stats.expired += 1;
                g.stats.misses += 1;
                None
            }
            None => {
                g.stats.misses += 1;
                None
            }
        }
    }

    /// Peek without touching LRU order or hit/miss stats (diagnostics only).
    pub fn contains(&self, key: &K) -> bool {
        let now = self.clock.now_ms();
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.map
            .get(key)
            .is_some_and(|e| !e.expires_at_ms.is_some_and(|x| x <= now))
    }

    pub fn insert(&self, key: K, value: V) {
        let expires = self.ttl_ms.map(|t| self.clock.now_ms() + t);
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.order.retain(|k| k != &key);
        g.order.push_back(key.clone());
        g.map.insert(
            key,
            Entry {
                value,
                expires_at_ms: expires,
            },
        );
        while g.map.len() > self.capacity {
            match g.order.pop_front() {
                Some(k) => {
                    g.map.remove(&k);
                    g.stats.evictions += 1;
                }
                None => break,
            }
        }
    }

    pub fn remove(&self, key: &K) -> bool {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let had = g.map.remove(key).is_some();
        g.order.retain(|k| k != key);
        had
    }

    pub fn invalidate_all(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        g.map.clear();
        g.order.clear();
    }

    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map
            .len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn stats(&self) -> CacheStats {
        let g = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        CacheStats {
            hits: g.stats.hits,
            misses: g.stats.misses,
            expired: g.stats.expired,
            evictions: g.stats.evictions,
            len: g.map.len(),
            capacity: self.capacity,
        }
    }
}

impl<K: Clone + Eq + Hash, V: Clone> BoundedCache<K, V> {
    /// Convenience for production call sites (system clock, real durations).
    pub fn with_system_clock(capacity: usize, ttl: Option<Duration>) -> Self {
        Self::new(capacity, ttl, Arc::new(SystemClock::default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn cache(cap: usize, ttl: u64) -> (BoundedCache<String, u32>, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new());
        (
            BoundedCache::new(cap, Some(Duration::from_millis(ttl)), clock.clone()),
            clock,
        )
    }

    #[test]
    fn bounded_insert_evicts_lru_front_never_grows() {
        let (c, _clk) = cache(2, 10_000);
        c.insert("a".into(), 1);
        c.insert("b".into(), 2);
        let _ = c.get(&"a".to_string()); // touch a ⇒ b is now LRU
        c.insert("c".into(), 3);
        assert_eq!(c.len(), 2, "hard capacity");
        assert_eq!(c.get(&"a".to_string()), Some(1));
        assert_eq!(c.get(&"b".to_string()), None, "LRU victim evicted");
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn ttl_expiry_counts_as_miss_and_removes() {
        let (c, clk) = cache(4, 100);
        c.insert("k".into(), 7);
        assert_eq!(c.get(&"k".to_string()), Some(7));
        clk.advance(Duration::from_millis(99));
        assert_eq!(c.get(&"k".to_string()), Some(7), "still inside ttl");
        clk.advance(Duration::from_millis(2));
        assert_eq!(c.get(&"k".to_string()), None);
        let s = c.stats();
        assert_eq!(s.expired, 1);
        assert_eq!(s.misses, 1);
        assert_eq!(s.hits, 2, "the in-ttl re-read was a hit");
        assert_eq!(c.len(), 0, "expired entry removed, not lingering");
    }

    #[test]
    fn zero_capacity_always_misses_but_never_panics() {
        let (c, _clk) = cache(0, 100);
        c.insert("x".into(), 1);
        assert_eq!(c.len(), 0);
        assert_eq!(c.get(&"x".to_string()), None);
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn hit_ratio_is_honest_at_idle() {
        let (c, _clk) = cache(1, 1000);
        assert_eq!(c.stats().hit_ratio(), 0.0, "idle cache is 0, not NaN/1");
        let _ = c.get(&"a".to_string());
        assert_eq!(c.stats().hit_ratio(), 0.0);
        c.insert("a".into(), 1);
        let _ = c.get(&"a".to_string());
        let _ = c.get(&"a".to_string());
        assert!((c.stats().hit_ratio() - 2.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn contains_and_invalidate_are_side_effect_free_for_stats() {
        let (c, _clk) = cache(2, 1000);
        c.insert("a".into(), 1);
        assert!(c.contains(&"a".to_string()));
        assert_eq!(c.stats().hits, 0, "peek is not a hit");
        c.invalidate_all();
        assert!(c.is_empty());
        assert_eq!(c.stats().misses, 0);
    }
}
