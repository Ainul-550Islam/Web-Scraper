use super::*;

#[test]
fn cancel_token_shares_engine_flag() {
    let flag = Arc::new(AtomicBool::new(false));
    let tok = CancelToken::from_shared(flag.clone());
    assert!(!tok.is_cancelled());
    flag.store(true, Ordering::Relaxed);
    assert!(tok.is_cancelled(), "external flip is observed");
    let t2 = tok.clone();
    t2.cancel();
    assert!(
        flag.load(Ordering::Relaxed),
        "shared flag sees clone's cancel"
    );
}

#[tokio::test]
async fn supervisor_tracks_states_and_counts_jobs() {
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    let _ = sup.register(0);
    let _ = sup.register(1);
    sup.note_job_done(0);
    sup.note_job_done(0);
    sup.note_job_failed(1);
    sup.note_idle(1);
    let s = sup.snapshot();
    assert_eq!(s.registered, 2);
    assert_eq!(s.workers[0].jobs_done, 2);
    assert_eq!(s.workers[1].jobs_failed, 1);
    assert_eq!(s.workers[1].state, WorkerState::Idle);
    assert_eq!(s.workers[0].state, WorkerState::Running);
    assert!((s.utilization - 0.5).abs() < f64::EPSILON);
}

#[tokio::test]
async fn drain_flag_is_global_and_idempotent() {
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    assert!(!sup.is_draining());
    sup.begin_drain();
    assert!(sup.is_draining());
    sup.begin_drain();
    assert!(sup.snapshot().draining);
}

#[tokio::test]
async fn panic_without_policy_reports_legacy_message() {
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    let err = sup
        .supervise(7, || async { panic!("boom") })
        .await
        .expect_err("must propagate");
    assert!(err.contains("worker task died"), "{err}");
    assert_eq!(sup.snapshot().deaths, 1);
    assert_eq!(sup.snapshot().total_restarts, 0);
}

#[tokio::test]
async fn restart_policy_restarts_bounded_then_gives_up() {
    let n = Arc::new(AtomicUsize::new(0));
    let sup = Supervisor::new(
        RestartPolicy {
            max_restarts: 2,
            backoff: Duration::from_millis(1),
        },
        CancelToken::new(),
    );
    let n2 = n.clone();
    let err = sup
        .supervise(0, move || {
            let n = n2.clone();
            async move {
                n.fetch_add(1, Ordering::SeqCst);
                panic!("always dies");
            }
        })
        .await
        .expect_err("exhausted restarts must report");
    assert!(err.contains("worker task died"), "{err}");
    assert_eq!(n.load(Ordering::SeqCst), 3, "initial + 2 restarts");
    assert_eq!(sup.snapshot().total_restarts, 2);
}

#[tokio::test]
async fn healthy_task_returns_ok_once() {
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    let done = Arc::new(AtomicBool::new(false));
    let d = done.clone();
    sup.supervise(0, move || {
        let d = d.clone();
        async move { d.store(true, Ordering::SeqCst) }
    })
    .await
    .expect("clean exit");
    assert!(done.load(Ordering::SeqCst));
}

#[tokio::test]
async fn taskset_caps_replaces_and_shuts_down() {
    let set = Arc::new(TaskSet::new(2));
    assert!(set.spawn("a", std::future::pending::<()>()));
    assert!(set.spawn("b", std::future::pending::<()>()));
    assert!(
        !set.spawn("c", std::future::pending::<()>()),
        "cap enforced"
    );
    // replacing an existing name is allowed even at cap
    assert!(set.spawn("a", std::future::pending::<()>()));
    assert_eq!(set.len(), 2);
    // a finishing task
    let finished = Arc::new(AtomicBool::new(false));
    let f = finished.clone();
    assert!(set.spawn("b", async move { f.store(true, Ordering::SeqCst) }));
    let stragglers = set.shutdown(Duration::from_secs(1)).await;
    assert_eq!(stragglers, vec!["a".to_string()], "pending task reported");
    assert!(finished.load(Ordering::SeqCst));
}

#[tokio::test]
async fn taskset_drop_aborts_orphans() {
    let ticks = Arc::new(AtomicUsize::new(0));
    {
        let t = ticks.clone();
        let set = TaskSet::new(4);
        set.spawn("ticker", async move {
            loop {
                t.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        // drop: must abort, and the counter must stop rising
    }
    let after = ticks.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        ticks.load(Ordering::SeqCst),
        after,
        "no orphan loop survived"
    );
    assert!(after >= 3, "ticker ran: {after}");
}

#[test]
fn registry_snapshot_serializes_worker_fields() {
    let sup = Supervisor::new(RestartPolicy::default(), CancelToken::new());
    let _ = sup.register(0);
    let v = serde_json::to_value(sup.snapshot()).unwrap();
    assert_eq!(v["workers"][0]["id"], 0);
    assert_eq!(v["workers"][0]["state"], "running");
    assert!(v.get("deaths").is_some());
}
