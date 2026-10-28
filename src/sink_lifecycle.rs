//! Owned sink workers and aggregate (not per-record) delivery deadlines.
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const DRAIN_LIMIT: Duration = Duration::from_secs(15);
pub(crate) const OP_LIMIT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct DeliveryDeadline(Mutex<Option<Instant>>);
impl DeliveryDeadline {
    pub(crate) fn begin(&self) {
        let mut d = self.0.lock().unwrap_or_else(|p| p.into_inner());
        d.get_or_insert_with(|| Instant::now() + DRAIN_LIMIT);
    }
    pub(crate) fn end(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }
    pub(crate) fn remaining(&self, maximum: Duration) -> Duration {
        let d = self.0.lock().unwrap_or_else(|p| p.into_inner());
        d.map_or(maximum, |at| {
            maximum.min(at.saturating_duration_since(Instant::now()))
        })
    }
}
