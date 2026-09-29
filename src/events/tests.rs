use super::*;

use serde_json::json;

#[test]
fn ring_is_bounded_drops_oldest_and_counts() {
    let bus = Bus::new(16);
    for i in 0..100u64 {
        bus.publish(EventKind::JobSubmitted, Some(i), json!({"i": i}));
    }
    let s = bus.stats();
    assert_eq!(s.cap, 16);
    assert_eq!(s.ring_len, 16);
    assert_eq!(s.published, 100);
    assert_eq!(s.dropped, 84);
    assert_eq!(s.last_seq, 100);
    // Tail returns the *newest* 16, oldest-first, with dense seqs.
    let t = bus.tail(5);
    assert_eq!(t.len(), 5);
    assert_eq!(t[0].seq, 96);
    assert_eq!(t[4].seq, 100);
    assert_eq!(t[4].detail["i"], 99);
}

#[tokio::test]
async fn cursor_delivery_waits_then_resolves_in_order() {
    let bus = Bus::new(64);
    let b2 = bus.clone();
    let pubr = tokio::spawn(async move {
        for i in 0..3u64 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            b2.publish(EventKind::JobCompleted, Some(i), json!({}));
        }
    });
    let mut got = Vec::new();
    let mut cur = 0u64;
    for _ in 0..3 {
        if let Some(ev) = bus.next_after(cur).await {
            got.push(ev.seq);
            cur = ev.seq;
        }
    }
    pubr.await.expect("publisher");
    assert_eq!(got, vec![1, 2, 3], "in-order, no loss, no duplication");
}

#[tokio::test]
async fn lagging_subscriber_rejoins_after_gap() {
    let bus = Bus::new(16);
    for i in 0..40u64 {
        bus.publish(EventKind::JobStarted, None, json!({"i": i}));
    }
    // Cursor way behind the retained window: must still return *some*
    // current event rather than wait forever for 2..40 (already gone).
    let ev = tokio::time::timeout(std::time::Duration::from_secs(1), bus.next_after(1))
        .await
        .expect("rejoins immediately")
        .expect("event");
    assert!(ev.seq > 1, "rejoin: {}", ev.seq);
}

#[tokio::test]
async fn close_releases_waiters_with_none() {
    let bus = Bus::new(16);
    let b2 = bus.clone();
    let waiter = tokio::spawn(async move { b2.next_after(7).await });
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    bus.close();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("waiter released")
            .expect("waiter task joined")
            .is_none()
    );
    assert!(bus.stats().closed);
}

#[test]
fn publish_after_close_still_counts_and_is_readable() {
    let bus = Bus::new(16);
    bus.close();
    let seq = bus.publish(EventKind::ShutdownCompleted, None, json!({"late": true}));
    assert_eq!(seq, 1);
    assert_eq!(bus.tail(4).len(), 1, "tail is for snapshots, not waits");
}

#[test]
fn kinds_stable_names_serialization() {
    assert_eq!(
        EventKind::SinkWriteCompleted.as_str(),
        "sink_write_completed"
    );
    let v = serde_json::to_value(EventKind::ProxyFailure).unwrap();
    assert_eq!(v, json!("proxy_failure"));
    let ev = Event {
        seq: 1,
        ts: 2,
        kind: EventKind::RequestStarted,
        job: None,
        detail: Value::Null,
    };
    let s = serde_json::to_string(&ev).unwrap();
    assert!(!s.contains("detail"), "null detail omitted: {s}");
    assert!(!s.contains("job"), "absent job omitted: {s}");
}

#[test]
fn cap_floor_prevents_zero_capacity_bus() {
    let bus = Bus::new(0); // misconfigured: clamp to usable floor
    assert!(bus.stats().cap >= 16);
    bus.publish(EventKind::QueueOverflow, None, json!({}));
    assert_eq!(bus.tail(1).len(), 1);
}
