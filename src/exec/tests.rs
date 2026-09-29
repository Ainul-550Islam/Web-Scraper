use super::*;

#[test]
fn deadline_math_is_monotone() {
    let plan = RequestPlan::new(Url::parse("http://x/a").unwrap())
        .with_deadline(Instant::now() + Duration::from_millis(50));
    assert!(!plan.expired(Instant::now()));
    assert!(plan.remaining(Instant::now()).unwrap() <= Duration::from_millis(60));
    assert!(plan.expired(Instant::now() + Duration::from_secs(1)));
    assert_eq!(
        plan.remaining(Instant::now() + Duration::from_secs(1)),
        Some(Duration::ZERO),
        "remaining never goes negative"
    );
}

#[test]
fn plan_builder_defaults_are_inert() {
    let plan = RequestPlan::new(Url::parse("http://x/b").unwrap());
    assert!(plan.headers.is_empty());
    assert!(plan.deadline.is_none());
    assert!(!plan.expired(Instant::now() + Duration::from_secs(3600)));
}

#[test]
fn context_carries_join_keys() {
    let ctx = RequestContext {
        correlation_id: 42,
        job_id: Some(JobId(7)),
        target: 1,
        host: "example.com".into(),
        bus: None,
        acct: None,
        budget: None,
    };
    assert_eq!(ctx.correlation_id, 42);
    assert_eq!(ctx.job_id.map(|j| j.0), Some(7));
}

#[test]
fn publish_is_silent_without_bus_and_counted_with_one() {
    let ctx = RequestContext {
        correlation_id: 1,
        job_id: Some(JobId(9)),
        target: 0,
        host: "h".into(),
        bus: Some(Bus::new(16)),
        acct: None,
        budget: None,
    };
    ctx.publish(
        crate::events::EventKind::RequestStarted,
        serde_json::json!({}),
    );
    let t = ctx.bus.as_ref().unwrap().tail(4);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].job, Some(9));
    // and the inert default never panics
    RequestContext::default().publish(
        crate::events::EventKind::RequestStarted,
        serde_json::json!({}),
    );
}
