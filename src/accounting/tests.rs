use super::*;

#[test]
fn counters_move_with_lifecycle() {
    let a = Accounting::new();
    for i in 0..5 {
        a.note_submit();
        a.note_start();
        if i % 2 == 0 {
            a.note_finish_ok(Duration::from_millis(10));
        } else {
            a.note_finish_failed(&format!("http://x/{i}"), "failed");
        }
    }
    let s = a.snapshot();
    assert_eq!(s.jobs_submitted, 5);
    assert_eq!(s.jobs_completed, 3);
    assert_eq!(s.jobs_failed, 2);
    assert_eq!(s.jobs_active, 0, "every start paired with a finish");
    assert_eq!(s.requests, 5);
}

#[test]
fn deferred_jobs_release_worker_capacity_without_counting_as_finished() {
    let a = Accounting::new();
    a.note_submit();
    a.note_start();
    a.note_deferred();
    let snapshot = a.snapshot();
    assert_eq!(snapshot.jobs_active, 0);
    assert_eq!(snapshot.jobs_completed, 0);
    assert_eq!(snapshot.jobs_failed, 0);
    assert_eq!(snapshot.jobs_submitted, 1);
}

#[test]
fn failure_ring_bounded_newest_wins() {
    let a = Accounting::new();
    for i in 0..(FAILURE_RING_CAP + 40) {
        a.note_start();
        a.note_finish_failed(&format!("http://x/{i}"), "boom");
    }
    let s = a.snapshot();
    assert_eq!(s.recent_failures.len(), FAILURE_RING_CAP);
    assert_eq!(
        s.recent_failures.last().map(|f| f.url.as_str()),
        Some("http://x/103")
    );
    assert_eq!(
        s.recent_failures.first().map(|f| f.url.as_str()),
        Some("http://x/40")
    );
}

#[test]
fn retry_history_bounded_and_counted() {
    let a = Accounting::new();
    for i in 0..100 {
        a.note_retry(i);
    }
    let s = a.snapshot();
    assert_eq!(s.retries, 100);
    assert_eq!(s.recent_retry_delays_ms.len(), RETRY_DELAYS_CAP);
    assert_eq!(s.recent_retry_delays_ms.last().copied(), Some(99));
}

#[test]
fn active_jobs_saturate_at_zero() {
    let a = Accounting::new();
    a.note_finish_ok(Duration::ZERO); // unpaired release
    assert_eq!(a.snapshot().jobs_active, 0, "never negative");
}

#[test]
fn bytes_and_expired_cancelled() {
    let a = Accounting::new();
    a.note_bytes(1024);
    a.note_bytes(3 * 1024);
    a.note_expired();
    a.note_cancelled();
    let s = a.snapshot();
    assert_eq!(s.bytes_received, 4 * 1024);
    assert_eq!(s.jobs_expired, 1);
    assert_eq!(s.jobs_cancelled, 1);
}

#[test]
fn throughput_and_wait_averages_are_finite() {
    let a = Accounting::new();
    a.note_start();
    a.note_finish_ok(Duration::from_millis(25));
    let s = a.snapshot();
    assert!(s.jobs_per_sec.is_finite());
    assert!(s.avg_queue_wait_ms >= 24.9, "{}", s.avg_queue_wait_ms);
}

#[test]
fn concurrent_updates_never_lose_a_count() {
    let a = Accounting::new();
    let mut handles = Vec::new();
    for _ in 0..8 {
        let a = a.clone();
        handles.push(std::thread::spawn(move || {
            for _ in 0..2000 {
                a.note_bytes(1);
                a.note_submit();
            }
        }));
    }
    for h in handles {
        h.join().expect("thread");
    }
    let s = a.snapshot();
    assert_eq!(s.bytes_received, 16_000);
    assert_eq!(s.jobs_submitted, 16_000);
}

#[tokio::test]
async fn snapshot_serializes_for_admin() {
    let a = Accounting::new();
    let v = serde_json::to_value(a.snapshot()).expect("json");
    for key in [
        "jobs_active",
        "bytes_received",
        "recent_failures",
        "jobs_per_sec",
    ] {
        assert!(v.get(key).is_some(), "missing {key}");
    }
}
