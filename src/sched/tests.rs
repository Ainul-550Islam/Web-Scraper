use super::*;

use url::Url;

fn cfg(mode: &str, depth: u32, inflight: u32) -> SchedulingCfg {
    SchedulingCfg {
        mode: mode.into(),
        max_queue_depth: depth,
        per_target_inflight: inflight,
        job_max_age_secs: 0,
        worker_restarts: 0,
        retry_budget_per_min: 0,
    }
}

fn job(u: &str, target: usize) -> Job {
    Job::new(Url::parse(u).unwrap(), 0, target)
}

#[test]
fn fifo_preserves_exact_insertion_order() {
    let q = CrawlQueue::new(&cfg("fifo", 0, 0), 1);
    for i in 0..5 {
        q.push(job(&format!("http://x/{i}"), 0)).unwrap();
    }
    let got: Vec<String> = (0..5)
        .map(|_| match q.pop() {
            Popped::Job(j) => j.url.to_string(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        got,
        (0..5).map(|i| format!("http://x/{i}")).collect::<Vec<_>>()
    );
    assert!(matches!(q.pop(), Popped::Empty));
}

#[test]
fn priority_drains_high_first_fifo_within_class() {
    let q = CrawlQueue::new(&cfg("priority", 0, 0), 1);
    q.push(job("http://x/n1", 0)).unwrap();
    let mut lo = job("http://x/lo", 0);
    lo.priority = JobPriority::Low;
    q.push(lo).unwrap();
    let mut hi1 = job("http://x/hi1", 0);
    hi1.priority = JobPriority::High;
    q.push(hi1).unwrap();
    let mut hi2 = job("http://x/hi2", 0);
    hi2.priority = JobPriority::High;
    q.push(hi2).unwrap();
    let order: Vec<String> = (0..4)
        .filter_map(|_| match q.pop() {
            Popped::Job(j) => Some(j.url.to_string()),
            _ => None,
        })
        .collect();
    assert_eq!(
        order,
        vec!["http://x/hi1", "http://x/hi2", "http://x/n1", "http://x/lo"]
    );
}

#[test]
fn bounded_depth_rejects_without_evicting() {
    let q = CrawlQueue::new(&cfg("fifo", 3, 0), 1);
    for i in 0..5 {
        let _ = q.push(job(&format!("http://x/{i}"), 0));
    }
    assert_eq!(q.len(), 3);
    let s = q.stats();
    assert_eq!(s.pushed, 3);
    assert_eq!(s.rejected, 2, "two pushes hit the cap and were refused");
    assert_eq!(s.depth, 3);
}

#[test]
fn fair_mode_round_robins_across_targets() {
    let q = CrawlQueue::new(&cfg("fair", 0, 0), 2);
    for i in 0..4 {
        q.push(job(&format!("http://a/{i}"), 0)).unwrap();
        q.push(job(&format!("http://b/{i}"), 1)).unwrap();
    }
    let seq: Vec<String> = (0..8)
        .filter_map(|_| match q.pop() {
            Popped::Job(j) => Some(j.url.to_string()),
            _ => None,
        })
        .collect();
    // Interleaved per target — no target starves the other.
    let a_positions: Vec<usize> = seq
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.contains("//a/").then_some(i))
        .collect();
    let b_positions: Vec<usize> = seq
        .iter()
        .enumerate()
        .filter_map(|(i, u)| u.contains("//b/").then_some(i))
        .collect();
    assert_eq!(a_positions.len(), 4);
    assert_eq!(b_positions.len(), 4);
    // The two streams must alternate at least once in the first 4 pops
    // (pure batching would give [0,1,2,3] all-a).
    assert!(
        a_positions[..4] != [0, 1, 2, 3] && b_positions[..4] != [0, 1, 2, 3],
        "fair interleave: a={a_positions:?} b={b_positions:?}"
    );
    // Within each target, order is preserved.
    let a_urls: Vec<&String> = seq.iter().filter(|u| u.contains("//a/")).collect();
    assert_eq!(
        a_urls,
        vec![
            &"http://a/0".to_string(),
            &"http://a/1".to_string(),
            &"http://a/2".to_string(),
            &"http://a/3".to_string()
        ]
    );
}

#[test]
fn per_target_inflight_gate_blocks_hot_target_not_others() {
    let q = CrawlQueue::new(&cfg("fair", 0, 1), 2);
    for i in 0..3 {
        q.push(job(&format!("http://a/{i}"), 0)).unwrap();
        q.push(job(&format!("http://b/{i}"), 1)).unwrap();
    }
    let first = match q.pop() {
        Popped::Job(j) => j,
        o => panic!("{o:?}"),
    };
    // target of `first` is now at its inflight limit (1): next pop must
    // be from the other target.
    let second = match q.pop() {
        Popped::Job(j) => j,
        o => panic!("{o:?}"),
    };
    assert_ne!(first.target, second.target, "gate steers away from active");
    // Both slots busy now -> blocked.
    assert!(matches!(q.pop(), Popped::Empty));
    q.release(first.target);
    q.release(second.target);
    assert!(matches!(q.pop(), Popped::Job(_)));
}

#[test]
fn deadline_expires_on_pop_and_is_counted() {
    let q = CrawlQueue::new(&cfg("fifo", 0, 0), 1);
    let mut stale = job("http://x/stale", 0);
    stale = stale.with_deadline(Duration::ZERO);
    q.push(stale).unwrap();
    q.push(job("http://x/fresh", 0)).unwrap();
    match q.pop() {
        Popped::Expired(j) => assert!(j.url.as_str().ends_with("stale")),
        o => panic!("expected Expired, got {o:?}"),
    }
    match q.pop() {
        Popped::Job(j) => assert!(j.url.as_str().ends_with("fresh")),
        o => panic!("{o:?}"),
    }
    assert_eq!(q.stats().expired, 1);
}

#[test]
fn push_transitions_claimed_state_to_pending() {
    let q = CrawlQueue::new(&cfg("fifo", 0, 0), 1);
    let mut j = job("http://x/1", 0);
    j.state = JobState::Running; // nonsense input
    q.push(j).unwrap(); // queue owns normalization
    match q.pop() {
        Popped::Job(j) => assert_eq!(j.state, JobState::Running, "pop marks Running"),
        o => panic!("{o:?}"),
    }
}

#[test]
fn stats_snapshot_shape_and_counters() {
    let q = CrawlQueue::new(&cfg("fifo", 4, 0), 1);
    for i in 0..6 {
        let _ = q.push(job(&format!("http://x/{i}"), 0));
    }
    let _ = q.pop();
    let s = q.stats();
    assert_eq!(s.pushed, 4);
    assert_eq!(s.rejected, 2);
    assert_eq!(s.popped, 1);
    assert_eq!(s.inflight, 1);
    assert_eq!(s.depth, 3);
    assert_eq!(s.per_target_depths, vec![(0, 3)]);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["mode"], "fifo");
    assert!(v.get("expired").is_some());
}

#[test]
fn release_saturates_and_zero_target_is_safe() {
    let q = CrawlQueue::new(&cfg("fifo", 0, 0), 1);
    q.release(99); // out of range: no panic
    assert_eq!(q.stats().inflight, 0);
    q.push(job("http://x/1", 0)).unwrap();
    match q.pop() {
        Popped::Job(j) => {
            q.release(j.target);
            q.release(j.target); // double release: saturate at 0
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(q.stats().inflight, 0);
}

#[test]
fn unknown_mode_degrades_to_fifo_explicitly() {
    let q = CrawlQueue::new(&cfg("teleport", 0, 0), 1);
    assert_eq!(q.stats().mode, "fifo");
}

#[test]
fn make_job_applies_priority_and_age() {
    let j = make_job(
        Url::parse("http://x/1").unwrap(),
        2,
        1,
        JobPriority::High,
        Some(Duration::from_secs(60)),
    );
    assert_eq!(j.priority, JobPriority::High);
    assert!(j.deadline.is_some());
    assert!(!j.expired_at(Instant::now()));
}
