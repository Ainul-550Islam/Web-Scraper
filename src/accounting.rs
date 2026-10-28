//! Resource accounting for a run: the numbers an operator checks when the
//! question is "what is the engine consuming and producing *right now*".
//!
//! Everything is atomics (single relaxed add per event on the hot path) or
//! a short-mutex ring (failure records). No allocation per job beyond the
//! bounded ring; counters never block or panic; utilization math is
//! computed at read time so the writer paths stay pure increments.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::events::now_secs;

/// One failure line for the runtime snapshot (bounded ring, newest last).
#[derive(Debug, Clone, Serialize)]
pub struct FailureRecord {
    pub ts: u64,
    /// Credential-redacted URL (callers pass through
    /// `config::redact_credentials` before recording).
    pub url: String,
    pub reason: String,
}

pub const FAILURE_RING_CAP: usize = 64;
pub const RETRY_DELAYS_CAP: usize = 32;

/// Run-scoped resource counters. Cheap to clone an Arc of this; shared by
/// engine workers, the fetcher (via request context) and admin.
#[derive(Debug)]
pub struct Accounting {
    started: Instant,
    started_ts: u64,
    /// Live job count per worker: +1 at pop, -1 at completion.
    active_jobs: AtomicI64,
    /// Live submission count in the queue (engine notes on push; the queue
    /// itself is authoritative for depth — this tracks submissions *made*,
    /// including ones already popped).
    jobs_submitted: AtomicU64,
    jobs_completed: AtomicU64,
    jobs_failed: AtomicU64,
    jobs_expired: AtomicU64,
    jobs_cancelled: AtomicU64,
    retries: AtomicU64,
    bytes_received: AtomicU64,
    requests: AtomicU64,
    recent_failures: Mutex<VecDeque<FailureRecord>>,
    recent_retry_delays_ms: Mutex<VecDeque<u64>>,
    /// Cumulative queue-wait for finished jobs (mean = /completed).
    queue_wait_ms_total: AtomicU64,
}

impl Default for Accounting {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            started_ts: now_secs(),
            active_jobs: AtomicI64::new(0),
            jobs_submitted: AtomicU64::new(0),
            jobs_completed: AtomicU64::new(0),
            jobs_failed: AtomicU64::new(0),
            jobs_expired: AtomicU64::new(0),
            jobs_cancelled: AtomicU64::new(0),
            retries: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            recent_failures: Mutex::new(VecDeque::new()),
            recent_retry_delays_ms: Mutex::new(VecDeque::new()),
            queue_wait_ms_total: AtomicU64::new(0),
        }
    }
}

impl Accounting {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            started: Instant::now(),
            started_ts: now_secs(),
            ..Default::default()
        })
    }

    pub fn note_submit(&self) {
        self.jobs_submitted.fetch_add(1, Ordering::Relaxed);
    }
    pub fn note_start(&self) {
        self.active_jobs.fetch_add(1, Ordering::SeqCst);
        self.requests.fetch_add(1, Ordering::Relaxed);
    }
    /// A running item was returned to the pending queue (usually shutdown or
    /// cancellation), so it no longer occupies a worker but is not complete.
    pub fn note_deferred(&self) {
        self.active_jobs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some((v - 1).max(0)))
            .ok();
    }
    pub fn note_finish_ok(&self, waited: Duration) {
        self.active_jobs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some((v - 1).max(0)))
            .ok();
        self.jobs_completed.fetch_add(1, Ordering::Relaxed);
        let _ = self.queue_wait_ms_total.fetch_add(
            waited.as_millis().try_into().unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }
    pub fn note_finish_failed(&self, url: &str, reason: &str) {
        self.active_jobs
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| Some((v - 1).max(0)))
            .ok();
        self.jobs_failed.fetch_add(1, Ordering::Relaxed);
        let mut g = self
            .recent_failures
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if g.len() >= FAILURE_RING_CAP {
            g.pop_front();
        }
        g.push_back(FailureRecord {
            ts: now_secs(),
            url: url.to_string(),
            reason: reason.to_string(),
        });
    }
    pub fn note_expired(&self) {
        self.jobs_expired.fetch_add(1, Ordering::Relaxed);
    }
    pub fn note_cancelled(&self) {
        self.jobs_cancelled.fetch_add(1, Ordering::Relaxed);
    }
    pub fn note_retry(&self, delay_ms: u64) {
        self.retries.fetch_add(1, Ordering::Relaxed);
        let mut g = self
            .recent_retry_delays_ms
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if g.len() >= RETRY_DELAYS_CAP {
            g.pop_front();
        }
        g.push_back(delay_ms);
    }
    pub fn note_bytes(&self, n: u64) {
        self.bytes_received.fetch_add(n, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> AccountingSnapshot {
        let elapsed = self.started.elapsed();
        let completed = self.jobs_completed.load(Ordering::Relaxed);
        AccountingSnapshot {
            started_ts: self.started_ts,
            elapsed_ms: elapsed.as_millis() as u64,
            jobs_active: self.active_jobs.load(Ordering::SeqCst).max(0) as u64,
            jobs_submitted: self.jobs_submitted.load(Ordering::Relaxed),
            jobs_completed: completed,
            jobs_failed: self.jobs_failed.load(Ordering::Relaxed),
            jobs_expired: self.jobs_expired.load(Ordering::Relaxed),
            jobs_cancelled: self.jobs_cancelled.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            requests: self.requests.load(Ordering::Relaxed),
            jobs_per_sec: if elapsed.is_zero() {
                0.0
            } else {
                completed as f64 / elapsed.as_secs_f64()
            },
            avg_queue_wait_ms: if completed == 0 {
                0.0
            } else {
                self.queue_wait_ms_total.load(Ordering::Relaxed) as f64 / completed as f64
            },
            recent_failures: self
                .recent_failures
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .cloned()
                .collect(),
            recent_retry_delays_ms: self
                .recent_retry_delays_ms
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .iter()
                .copied()
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountingSnapshot {
    pub started_ts: u64,
    pub elapsed_ms: u64,
    pub jobs_active: u64,
    pub jobs_submitted: u64,
    pub jobs_completed: u64,
    pub jobs_failed: u64,
    pub jobs_expired: u64,
    pub jobs_cancelled: u64,
    pub retries: u64,
    pub bytes_received: u64,
    pub requests: u64,
    pub jobs_per_sec: f64,
    pub avg_queue_wait_ms: f64,
    pub recent_failures: Vec<FailureRecord>,
    pub recent_retry_delays_ms: Vec<u64>,
}

#[cfg(test)]
mod tests;
