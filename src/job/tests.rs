use super::*;

fn job() -> Job {
    Job::new(Url::parse("http://x/1").unwrap(), 0, 0)
}

#[test]
fn ids_are_unique_and_monotonic() {
    let a = JobId::next();
    let b = JobId::next();
    assert!(b.0 > a.0, "{a:?} {b:?}");
}

#[test]
fn legal_lifecycle_walks() {
    let mut j = job();
    assert_eq!(j.state, JobState::Pending);
    j.transition(JobState::Running).unwrap();
    j.complete(JobResult {
        written: true,
        ..Default::default()
    });
    assert_eq!(j.state, JobState::Completed);
    assert!(j.state.is_terminal());
}

#[test]
fn illegal_transitions_rejected_with_detail() {
    let mut j = job();
    // Pending -> Completed is illegal (must run first)
    let err = j.transition(JobState::Completed).unwrap_err();
    assert_eq!(err.from, JobState::Pending);
    assert_eq!(err.to, JobState::Completed);
    assert_eq!(j.state, JobState::Pending, "rejection leaves state intact");
    // Terminal states are final.
    j.transition(JobState::Running).unwrap();
    j.fail(JobError {
        reason: "boom".into(),
        category: None,
        attempts_used: 2,
    });
    assert!(j.transition(JobState::Running).is_err());
    assert!(j.transition(JobState::Completed).is_err());
}

#[test]
fn deadline_expiry_is_time_relative_not_instant_constructed() {
    let mut j = job();
    assert!(!j.expired_at(Instant::now()));
    j = job().with_deadline(Duration::ZERO);
    assert!(j.expired_at(Instant::now()));
    j = job().with_deadline(Duration::from_secs(3600));
    assert!(!j.expired_at(Instant::now()));
}

#[test]
fn pending_jobs_can_be_cancelled_or_expired_running_cannot_expire() {
    assert!(JobState::can_transition(
        JobState::Pending,
        JobState::Cancelled
    ));
    assert!(JobState::can_transition(
        JobState::Pending,
        JobState::Expired
    ));
    assert!(!JobState::can_transition(
        JobState::Running,
        JobState::Expired
    ));
    assert!(JobState::can_transition(
        JobState::Running,
        JobState::Cancelled
    ));
}

#[test]
fn retry_meta_bounds_history_and_tracks_max_attempt() {
    let mut m = RetryMeta::default();
    for i in 1..=20u32 {
        m.note(i, i as u64 * 10, "transport");
    }
    assert_eq!(m.attempts, 20);
    assert_eq!(m.history.len(), RetryMeta::HISTORY_CAP);
    assert_eq!(m.history.last().map(|h| h.attempt), Some(20));
    assert_eq!(m.history.first().map(|h| h.attempt), Some(13));
}

#[test]
fn priority_rank_orders_high_above_low() {
    assert!(JobPriority::High.rank() > JobPriority::Normal.rank());
    assert!(JobPriority::Normal.rank() > JobPriority::Low.rank());
}

#[test]
fn status_report_reflects_live_state() {
    let mut j = job().with_priority(JobPriority::High);
    j.retry.note(1, 55, "transport");
    j.transition(JobState::Running).unwrap();
    let s = j.status_report();
    assert_eq!(s.state, JobState::Running);
    assert_eq!(s.priority, JobPriority::High);
    assert_eq!(s.attempts, 1);
}

#[test]
fn job_serde_round_trip_skips_instant_fields() {
    let j = job();
    let txt = serde_json::to_string(&j).unwrap();
    let back: Job = serde_json::from_str(&txt).unwrap();
    assert_eq!(back.id, j.id);
    assert_eq!(back.url, j.url);
    assert_eq!(back.state, JobState::Pending);
    // deadline is process-local (Instant) — not serialized by design.
    assert!(back.deadline.is_none());
}
