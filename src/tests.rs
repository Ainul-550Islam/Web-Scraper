//! Crate-surface tests: the module tree IS the public API contract
//! (docs.rs, downstream embedders, and the README layout all mirror
//! it). A module renamed or dropped from `lib.rs` is a breaking change

// and this test is the compiler-checked tripwire for it.

#[test]
fn every_engine_module_is_wired_into_the_crate_tree() {
    // One type per module — instantiating the names is enough; this test
    // exists to fail at COMPILE time if the tree changes shape.
    #[allow(unused_imports)]
    use {
        crate::accounting::Accounting, crate::admin::AdminState, crate::alerts::evaluate,
        crate::api::ApiError, crate::audit::Recorder, crate::breaker::Breaker,
        crate::ckpt::Checkpoint, crate::config::Config, crate::engine::Summary,
        crate::events::EventKind, crate::exec::RequestPlan, crate::export::ExportOpts,
        crate::extract::Extractor, crate::fetcher::FetchError, crate::job::Job,
        crate::metrics::Metrics, crate::ops::OpsLog, crate::policy::Politeness,
        crate::proxy::ProxyPool, crate::retry::RetryBudget, crate::reval::Store,
        crate::robots::Robots, crate::runtime::Runtime, crate::sched::CrawlQueue,
        crate::schema::Schema, crate::sign::Key, crate::sink::Record, crate::sitemap::Sitemap,
        crate::supervisor::Supervisor, crate::testutil::MockServer, crate::transform::Pipeline,
    };
    // runner + engine entry points are functions — reference them so the
    // paths must resolve.
    let _: fn() = || {
        let _ = crate::engine::run;
        let _ = crate::runner::serve;
        let _ = crate::admin::serve;
    };
    #[cfg(feature = "redis")]
    let _: fn() = || {
        let _ = crate::dist::worker;
    };
    #[cfg(feature = "postgres")]
    let _: fn() = || {
        let _ = <crate::pgsink::PgSink as crate::sink::Sink>::flush;
    };
}

#[test]
fn reexports_are_aliases_not_copies() {
    // `harvest::Config` and `harvest::config::Config` must be the SAME
    // type (one definition each; no shim structs that can drift).
    fn same<T>(_a: &T, _b: &T) {}
    let a: crate::Config = Default::default();
    let b: crate::config::Config = Default::default();
    same(&a, &b);
    let s1: crate::Summary = Default::default();
    let s2: crate::engine::Summary = Default::default();
    same(&s1, &s2);
}

#[test]
fn event_kinds_serialization_matches_as_str() {
    // /events and events.jsonl consumers use the serde name; `as_str`
    // is the CLI/grep name. They must agree, or log tooling breaks.
    use crate::events::EventKind::*;
    let kinds = [
        JobSubmitted,
        JobStarted,
        RequestStarted,
        RequestFinished,
        RetryScheduled,
        ProxySelected,
        ProxyFailure,
        ExtractionCompleted,
        SinkWriteCompleted,
        JobCompleted,
        JobFailed,
        ShutdownStarted,
        ShutdownCompleted,
        JobExpired,
        QueueOverflow,
        BreakerOpened,
        BreakerClosed,
        RecordQuarantined,
        OperatorAction,
    ];
    for k in kinds {
        let json = serde_json::to_string(&k).unwrap();
        let want = format!("\"{}\"", k.as_str());
        assert_eq!(json, want, "serde name drifted for {k:?}");
    }
}
