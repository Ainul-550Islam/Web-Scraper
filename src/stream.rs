//! Bounded fan-out stream for *observers* (dashboards, tail-writers, test
//! harnesses). Deliberately simple and synchronous: the engine's hot path is
//! the event ring in [`crate::events`]; this type exists so an arbitrary
//! number of external consumers can watch items without the publisher ever
//! waiting on them.
//!
//! Guarantees (each pinned by a unit test):
//! * **bounded**: every subscriber has a fixed-capacity queue;
//! * **backpressure is per-subscriber policy**, not publisher state —
//!   `LagDropOldest` (continuous-view semantics) or `RejectNew`
//!   (audit-grade: you are told exactly how many items you missed);
//! * **isolation**: a slow or dead subscriber can only ever degrade its own
//!   delivery counters; other subscribers and the publisher are unaffected;
//! * **cancellation**: `close()` is final — publish returns zero delivery and
//!   subscribers drain, then see end-of-stream; `unsubscribe` frees the slot;
//! * **no deadlock by construction**: publish never blocks (no condition
//!   variables, no async), so a stalled observer cannot stall a run.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backpressure {
    /// Drop this subscriber's OLDEST unread item to admit the new one.
    LagDropOldest,
    /// Drop the NEW item for this subscriber only (count it as lagged).
    RejectNew,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PublishSummary {
    /// number of subscribers that received this item
    pub delivered: usize,
    /// total items dropped across all subscribers due to full queues
    pub dropped: usize,
}

struct Slot<T> {
    policy: Backpressure,
    queue: VecDeque<T>,
    dropped: Arc<AtomicU64>,
}

type Slots<T> = HashMap<u64, Slot<T>>;

struct StreamInner<T> {
    capacity: usize,
    slots: Mutex<Slots<T>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Handle held by a consumer. Items drain via [`Self::recv`] (or its peeking
/// sibling for windowed views); `dropped()` counts what this subscriber's
/// bounded queue could not hold for THIS subscriber only.
pub struct Subscriber<T> {
    id: u64,
    inner: Arc<StreamInner<T>>,
    dropped: Arc<AtomicU64>,
    closed: Arc<AtomicBool>,
}

impl<T: Clone> Subscriber<T> {
    /// Next item, or `None` when the queue is empty. Never blocks — callers
    /// poll on their own cadence, like `/events` polling does.
    pub fn recv(&mut self) -> Option<T> {
        let mut g = lock(&self.inner.slots);
        let slot = g.get_mut(&self.id)?;
        let item = slot.queue.pop_front();
        if item.is_none() && self.closed.load(Ordering::Relaxed) {
            g.remove(&self.id); // closed + drained ⇒ end of stream
        }
        item
    }

    /// Newest unread item (bounded "latest state" readers); does not consume.
    pub fn peek_latest(&self) -> Option<T> {
        let g = lock(&self.inner.slots);
        g.get(&self.id)?.queue.back().cloned()
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
    pub fn id(&self) -> u64 {
        self.id
    }
    /// Give up the slot; the stream frees its memory immediately.
    pub fn unsubscribe(self) {
        lock(&self.inner.slots).remove(&self.id);
    }
}

impl<T> Drop for Subscriber<T> {
    fn drop(&mut self) {
        if let Ok(mut g) = self.inner.slots.lock() {
            g.remove(&self.id);
        }
    }
}

/// One open stream; cloneable producer handle. Items are whatever the
/// producer picks (`Arc<Value>` for JSON event fan-out, small structs).
pub struct Stream<T> {
    inner: Arc<StreamInner<T>>,
    closed: Arc<AtomicBool>,
    next_id: AtomicU64,
}

impl<T: Clone> Stream<T> {
    /// `capacity >= 1` is enforced: a zero-slot stream would be indistinguishable from a broken
    /// invariant; a kill-switch belongs to [`Stream::close`], not to `capacity = 0`.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(StreamInner {
                capacity: capacity.max(1),
                slots: Mutex::new(HashMap::new()),
            }),
            closed: Arc::new(AtomicBool::new(false)),
            next_id: AtomicU64::new(1),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }

    /// Register a subscriber BEFORE publishing what matters to it.
    pub fn subscribe(&self, policy: Backpressure) -> Subscriber<T> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let dropped = Arc::new(AtomicU64::new(0));
        lock(&self.inner.slots).insert(
            id,
            Slot {
                policy,
                queue: VecDeque::new(),
                dropped: dropped.clone(),
            },
        );
        Subscriber {
            id,
            inner: self.inner.clone(),
            dropped,
            closed: self.closed.clone(),
        }
    }

    pub fn publish(&self, item: T) -> PublishSummary {
        if self.is_closed() {
            return PublishSummary::default();
        }
        let mut g = lock(&self.inner.slots);
        let mut dropped_total = 0usize;
        let mut delivered = 0usize;
        for slot in g.values_mut() {
            let full = slot.queue.len() >= self.inner.capacity;
            match slot.policy {
                Backpressure::LagDropOldest => {
                    if full {
                        slot.queue.pop_front();
                        slot.dropped.fetch_add(1, Ordering::Relaxed);
                        dropped_total += 1;
                    }
                    slot.queue.push_back(item.clone());
                    delivered += 1;
                }
                Backpressure::RejectNew => {
                    if full {
                        slot.dropped.fetch_add(1, Ordering::Relaxed);
                        dropped_total += 1;
                    } else {
                        slot.queue.push_back(item.clone());
                        delivered += 1;
                    }
                }
            }
        }
        PublishSummary {
            delivered,
            dropped: dropped_total,
        }
    }

    /// Final for this stream: publishes become no-ops; subscribers drain, then
    /// see end-of-stream (clean shutdown, no loss of already-queued items).
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }

    pub fn subscriber_count(&self) -> usize {
        lock(&self.inner.slots).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_queue_drops_oldest_for_a_slow_laggy_subscriber() {
        let s: Stream<u32> = Stream::new(2);
        let mut sub = s.subscribe(Backpressure::LagDropOldest);
        for i in 0..5 {
            s.publish(i);
        }
        let got: Vec<_> = std::iter::from_fn(|| sub.recv()).collect();
        assert_eq!(got, vec![3, 4], "capacity 2 keeps the newest two");
        assert_eq!(sub.dropped(), 3, "0,1,2 were dropped for THIS subscriber");
    }

    #[test]
    fn reject_new_keeps_the_head_and_counts_lag() {
        let s: Stream<u32> = Stream::new(2);
        let mut sub = s.subscribe(Backpressure::RejectNew);
        for i in 0..5 {
            s.publish(i);
        }
        let got: Vec<_> = std::iter::from_fn(|| sub.recv()).collect();
        assert_eq!(got, vec![0, 1], "full queue rejects the new items");
        assert_eq!(sub.dropped(), 3);
    }

    #[test]
    fn slow_subscriber_cannot_starve_a_fast_one() {
        let s: Stream<u32> = Stream::new(1);
        let mut slow = s.subscribe(Backpressure::LagDropOldest); // never reads mid-run
        let mut fast = s.subscribe(Backpressure::LagDropOldest);
        for i in 0..10 {
            s.publish(i);
            let _ = fast.recv(); // keep up
        }
        assert_eq!(fast.recv(), None, "fast stays current");
        assert_eq!(
            slow.peek_latest(),
            Some(9),
            "slow still has the newest item"
        );
        assert_eq!(
            slow.dropped(),
            9,
            "and knows exactly what it missed (items 0..8 were overwritten)"
        );
        let _ = slow.recv();
    }

    #[test]
    fn publish_after_close_delivers_nothing_but_drain_works() {
        let s: Stream<u32> = Stream::new(4);
        let mut sub = s.subscribe(Backpressure::RejectNew);
        s.publish(1);
        s.close();
        let sum = s.publish(2);
        assert_eq!(sum.delivered, 0, "closed stream: publisher sees zero");
        assert_eq!(sub.recv(), Some(1), "pre-close item still readable");
        assert_eq!(sub.recv(), None, "then end-of-stream");
    }

    #[test]
    fn unsubscribe_frees_the_slot_and_never_panics_publisher() {
        let s: Stream<u32> = Stream::new(2);
        let sub = s.subscribe(Backpressure::LagDropOldest);
        assert_eq!(s.subscriber_count(), 1);
        sub.unsubscribe();
        assert_eq!(s.subscriber_count(), 0);
        let sum = s.publish(7); // zero subscribers: fine, not an error
        assert_eq!(sum, PublishSummary::default());
    }

    #[test]
    fn publish_is_never_blocked_by_a_full_bounded_slot() {
        let s: Stream<u32> = Stream::new(1);
        let _clogged = s.subscribe(Backpressure::LagDropOldest);
        for i in 0..1000 {
            let sum = s.publish(i);
            assert_eq!(sum.delivered, 1, "every publish delivered (by overwriting)");
        }
    }
}
