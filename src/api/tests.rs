use super::*;

#[test]
fn correlation_ids_validate_and_generate() {
    assert_eq!(
        CorrelationId::from_incoming(Some("job-42.a:b_c-9")).as_str(),
        "job-42.a:b_c-9"
    );
    for bad in ["", "with space", "crlf\r\nX", &"x".repeat(65), "quote\"id"] {
        let id = CorrelationId::from_incoming(Some(bad));
        assert!(id.as_str().starts_with("req-"), "reject {bad:?} -> {id}");
        assert!(CorrelationId::valid(id.as_str()));
    }
    // two consecutive generations differ
    assert_ne!(CorrelationId::new(), CorrelationId::new());
}

#[test]
fn query_parsing_bounds_and_rejects() {
    let q = Query::parse("limit=10&sort=ts&order=asc&x=%41%2b%20%42").unwrap();
    assert_eq!(q.get("limit"), Some("10"));
    assert_eq!(q.get("x"), Some("A+ B"));
    assert!(Query::parse(&"a=1&".repeat(40)).is_err());
    assert!(Query::parse(&format!("k={}", "v".repeat(300))).is_err());
    assert!(Query::parse("bad=%zz").is_err());
    assert!(
        Query::parse("k=lit%00null").is_err(),
        "decoded control byte rejected"
    );
    assert!(Query::parse("k=a%0Ab").is_err(), "raw newline via escape");
    // control chars smuggled without escaping cannot survive either
    assert!(Query::parse("k=a\nb").is_err());
}

#[test]
fn pagination_is_bounded_and_deterministic() {
    let p = Page::from_query(&Query::parse("limit=10000").unwrap()).unwrap();
    assert_eq!(p.limit, PAGE_MAX_LIMIT, "clamp, not reject");
    assert_eq!(p.offset, 0);
    assert_eq!(
        Page::from_query(&Query::parse("").unwrap()).unwrap().limit,
        PAGE_DEFAULT_LIMIT
    );
    assert!(Page::from_query(&Query::parse("limit=0").unwrap()).is_err());
    assert!(Page::from_query(&Query::parse("limit=x").unwrap()).is_err());
    assert!(Page::from_query(&Query::parse("offset=99999999").unwrap()).is_err());
    let items: Vec<u64> = (0..250).collect();
    let w = Page {
        limit: 200,
        offset: 0,
    }
    .window(items);
    assert_eq!(w.total, 250);
    assert_eq!(w.items.len(), 200);
    assert_eq!(w.items.first(), Some(&0));
}

fn run(index: u64, ts: u64, status: &str, duration: u64, written: u64) -> RunInfo {
    RunInfo {
        index,
        ts,
        run: "r".into(),
        status: status.into(),
        duration_ms: duration,
        config_sha256: None,
        written,
        quarantined: 0,
        error: None,
    }
}

#[test]
fn run_filtering_sorting_is_a_total_order() {
    let mut items = vec![
        run(0, 100, "ok", 5, 10),
        run(1, 200, "error", 5, 3),
        run(2, 200, "ok", 9, 0),
        run(3, 300, "cancelled", 1, 4),
    ];
    let f = ListFilter::from_query(&Query::parse("status=ok").unwrap()).unwrap();
    items.retain(|r| f.keep_run(r));
    assert_eq!(
        items.iter().map(|r| r.index).collect::<Vec<_>>(),
        vec![0, 2]
    );
    // stable tie on ts uses the history index (both ts=200 above)
    let mut all = vec![run(0, 200, "ok", 1, 0), run(1, 200, "ok", 1, 0)];
    let g = ListFilter {
        sort: Some((SortKey::Ts, SortDir::Desc)),
        status: None,
        since: None,
        until: None,
    };
    g.sort_runs(&mut all);
    assert_eq!(all[0].index, 1, "desc ties keep newest-first determinism");

    let h = ListFilter {
        sort: Some((SortKey::Written, SortDir::Asc)),
        status: None,
        since: None,
        until: None,
    };
    let mut m = vec![
        run(0, 1, "ok", 1, 5),
        run(1, 2, "ok", 1, 2),
        run(2, 3, "ok", 1, 5),
    ];
    h.sort_runs(&mut m);
    assert_eq!(
        m.iter().map(|r| r.index).collect::<Vec<_>>(),
        vec![1, 0, 2],
        "asc by written, ties by index"
    );

    let since = ListFilter::from_query(&Query::parse("since=150&until=250").unwrap()).unwrap();
    let items = [run(0, 100, "ok", 0, 0), run(1, 200, "ok", 0, 0)];
    assert!(items.iter().filter(|r| since.keep_run(r)).count() == 1);
    assert!(ListFilter::from_query(&Query::parse("since=5&until=1").unwrap()).is_err());
    assert!(ListFilter::from_query(&Query::parse("sort=bogus").unwrap()).is_err());
    assert!(ListFilter::from_query(&Query::parse("status=bogus").unwrap()).is_err());
    assert!(ListFilter::from_query(&Query::parse("sort=ts&order=sideways").unwrap()).is_err());
}

#[test]
fn run_info_degrades_on_odd_history_lines() {
    let r = RunInfo::from_history(&json!({"ts": "not-a-number"}), 7);
    assert_eq!(r.index, 7);
    assert_eq!(r.ts, 0);
    assert_eq!(r.status, "");
    assert_eq!(r.error, None);
    let r2 = RunInfo::from_history(
        &json!({"status":"error","error":"boom","summary":{"written":3,"quarantined":1},"config_sha256":"ab"}),
        1,
    );
    assert_eq!(r2.error.as_deref(), Some("boom"));
    assert_eq!(r2.written, 3);
    assert_eq!(r2.quarantined, 1);
    assert_eq!(r2.config_sha256.as_deref(), Some("ab"));
}

#[test]
fn health_report_aggregates_overall_and_sorts_names() {
    let h = HealthReport::build(vec![
        ComponentReport {
            name: "zeta",
            status: ComponentStatus::Disabled,
            detail: None,
        },
        ComponentReport {
            name: "alpha",
            status: ComponentStatus::Degraded,
            detail: Some(json!({"n": 2})),
        },
    ]);
    assert_eq!(h.overall, ComponentStatus::Degraded);
    assert_eq!(h.components[0].name, "alpha");
    // unavailable outranks degraded
    let h2 = HealthReport::build(vec![
        ComponentReport {
            name: "a",
            status: ComponentStatus::Unavailable,
            detail: None,
        },
        ComponentReport {
            name: "b",
            status: ComponentStatus::Degraded,
            detail: None,
        },
    ]);
    assert_eq!(h2.overall, ComponentStatus::Unavailable);
    // all-disabled flattens to healthy (nothing is broken)
    let h3 = HealthReport::build(vec![ComponentReport {
        name: "a",
        status: ComponentStatus::Disabled,
        detail: None,
    }]);
    assert_eq!(h3.overall, ComponentStatus::Healthy);
}

#[test]
fn queue_and_worker_views_are_honest_about_missing_runtime() {
    // null queue (engine running before a queue attached) => unavailable
    let q = QueueHealth::from_snapshot(&json!({"queue": null}), true);
    assert_eq!(q.state, ComponentStatus::Unavailable);
    assert!(q.stats.is_none());
    // attached queue passes the live stats through verbatim
    let snap = json!({"queue": {"depth": 3}, "workers": {"deaths": 2, "busy": 1, "registered": 2}, "accounting": {"avg_queue_wait_ms": 4.5}});
    let q = QueueHealth::from_snapshot(&snap, true);
    assert_eq!(q.state, ComponentStatus::Healthy);
    assert_eq!(q.stats.as_ref().unwrap()["depth"], 3);
    assert_eq!(q.avg_queue_wait_ms, 4.5);
    let w = WorkerHealth::from_snapshot(&snap, true);
    assert_eq!(
        w.state,
        ComponentStatus::Degraded,
        "worker deaths are signal"
    );
    let w2 = WorkerHealth::from_snapshot(&json!({"workers": null}), false);
    assert_eq!(w2.state, ComponentStatus::Unavailable);
    let w3 = WorkerHealth::from_snapshot(
        &json!({"workers": {"deaths": 0, "busy": 0, "registered": 2}}),
        false,
    );
    assert_eq!(w3.state, ComponentStatus::Healthy, "idle is normal");
}

#[test]
fn proxy_classification_is_ewma_banded() {
    let snap = |req: u64, succ: f64| crate::proxy::ProxySnapshot {
        inflight: 0,
        id: "p".into(),
        region: None,
        latency_ewma_ms: 1.0,
        success_ewma: succ,
        requests: req,
        failures: 0,
    };
    let info = ProxyEgressInfo::pool_info(vec![
        snap(0, 1.0),
        snap(10, 0.95),
        snap(10, 0.6),
        snap(10, 0.2),
    ]);
    assert_eq!(info.total, 4);
    assert_eq!(
        (info.unprobed, info.healthy, info.degraded, info.down),
        (1, 1, 1, 1)
    );
    // serialization must never carry a URL (ProxySnapshot has none, and
    // the API layer must not add one — pinned here).
    let j = serde_json::to_string(&info.entries[0]).unwrap();
    assert!(!j.contains("http"), "{j}");
}

#[test]
fn config_inspect_redacts_every_secret_channel() {
    let cfg = Config {
        admin_token: Some("s3cr3t-token-value".into()),
        alerts: crate::config::AlertCfg {
            webhook_url: Some("https://hooks.example.com/T/abcd?k=1".into()),
            ..Default::default()
        },
        redis_url: Some("redis://user:pass@127.0.0.1:6379/0".into()),
        ..Default::default()
    };
    let v = config_inspect(&cfg, None);
    let s = serde_json::to_string(&v).unwrap();
    assert!(!s.contains("s3cr3t-token-value"), "{s}");
    assert!(!s.contains("abcd"), "{s}");
    assert!(!s.contains("user:pass"), "{s}");
    assert_eq!(v["admin_token"], "<redacted>");
    assert!(v["_sources"]["name"] == "default");
    assert!(v["validation"] == "ok");
    assert!(v["backends"].is_object());
    // provenance: explicit + env markers from the raw file
    let raw = json!({"name": "override", "redis_url": "${REDIS}", "targets": []});
    let mut v2 = serde_json::to_value(&cfg).unwrap();
    redact_config(&mut v2, Some(&raw));
    assert_eq!(v2["_sources"]["name"], "explicit");
    assert_eq!(v2["_sources"]["redis_url"], "env:REDIS");
    assert_eq!(v2["_sources"]["concurrency"], "default");
}

#[test]
fn table_and_log_line_are_bounded_and_readable() {
    let rows: Vec<Vec<String>> = (0..1200).map(|i| vec![i.to_string(), "x".into()]).collect();
    let t = render_table(&["i", "v"], &rows);
    assert_eq!(
        t.lines().count(),
        1002,
        "header + 1000 rows + truncation note"
    );
    assert!(t.contains("200 more rows"));
    let long = render_table(&["a"], &[vec!["y".repeat(100)], vec!["short".into()]]);
    assert!(long.lines().nth(1).unwrap().chars().count() <= 65);
    let snap = json!({"engine":{"run":"r1","status":"idle","uptime_ms":42},"accounting":{"records_written":3,"records_quarantined":0},"events":{"published":7}});
    assert_eq!(
        snapshot_log_line(&snap),
        "run=r1 status=idle uptime_ms=42 written=3 quarantined=0 events=7"
    );
    assert!(snapshot_log_line(&json!({})).starts_with("run=-"));
}

#[test]
fn admin_client_rejects_illegal_targets_without_io() {
    // no connection is attempted for bad paths/addresses (fail before
    // touching the wire — a CLI typo must never reach a socket)
    let c = AdminClient::new("not-an-addr", None);
    assert!(c.get("/x").is_err());
    let c2 = AdminClient::new("127.0.0.1:1", None);
    assert!(c2.get("/x\r\nINJECT").is_err());
    assert!(c2.get(&format!("/{}", "p".repeat(2000))).is_err());
    // unreachable backend => 503-class ApiError, not a panic
    let e = c2.get("/health").unwrap_err();
    assert_eq!(e.status, 503);
}

#[test]
fn build_query_encodes_and_skips_absent() {
    let qs = build_query(&[
        ("limit", Some("5".into())),
        ("status", None),
        ("q", Some("a b&c".into())),
    ]);
    assert_eq!(qs, "limit=5&q=a%20b%26c");
    // round-trips through the parser
    let back = Query::parse(&qs).unwrap();
    assert_eq!(back.get("q"), Some("a b&c"));
}

#[test]
fn every_documented_knob_resolves() {
    // The explain table must not name dead paths: every knob has to
    // resolve inside the default serialized config.
    let v = serde_json::to_value(Config::default()).unwrap();
    for (path, ..) in CONFIG_KNOBS {
        let root = path.split('.').next().unwrap();
        if v.get(root).map(|x| x.is_null()).unwrap_or(false) {
            // optional value (null path, unset url, or absent object):
            // documented as nullable. Nested knobs of an optional
            // object resolve whenever that object is set (checked below).
            continue;
        }
        assert!(
            knob_lookup(&v, path).is_some(),
            "explain table references non-field `{path}`"
        );
    }
    let with_tb = json!({"transport_breaker": {"min_consecutive": 4, "cooldown_secs": 60, "recovery_successes": 2}});
    assert!(knob_lookup(&with_tb, "transport_breaker.min_consecutive").is_some());
    assert!(CONFIG_KNOBS.len() >= 40, "explain lost rows?");
    let rows = explain_config(&v, None, Some("alerts"));
    assert!(rows
        .iter()
        .all(|r| r["path"].as_str().unwrap().starts_with("alerts")));
    assert!(!rows.is_empty());
    let all = explain_config(&v, None, None);
    assert_eq!(all.len(), CONFIG_KNOBS.len());
}

#[test]
fn api_errors_have_stable_shapes() {
    for (e, line, code) in [
        (
            ApiError::unauthorized("nope"),
            "401 Unauthorized",
            "unauthorized",
        ),
        (ApiError::invalid("x"), "400 Bad Request", "invalid_request"),
        (ApiError::not_found("x"), "404 Not Found", "not_found"),
        (
            ApiError::method_not_allowed("DELETE", "/stats"),
            "405 Method Not Allowed",
            "method_not_allowed",
        ),
        (
            ApiError::payload_too_large(4096),
            "413 Payload Too Large",
            "payload_too_large",
        ),
        (
            ApiError::unavailable("x"),
            "503 Service Unavailable",
            "unavailable",
        ),
        (
            ApiError::internal(),
            "500 Internal Server Error",
            "internal",
        ),
    ] {
        assert_eq!(e.status_line(), line);
        assert_eq!(e.code, code);
        let j = e.to_json("req-1");
        assert_eq!(j["error"]["code"], code);
        assert_eq!(j["request_id"], "req-1");
    }
    // the 500 message is a fixed string: no internals can ride along
    assert_eq!(
        ApiError::internal().message,
        ApiError::internal().message.clone()
    );
    assert!(!ApiError::internal().message.contains("harvest::"));
}

#[test]
fn path_labels_redact_directory_layout() {
    let l = path_label(std::path::Path::new(
        "/srv/harvest/private/dataset.p1.jsonl",
    ));
    assert_eq!(l, "…/private/dataset.p1.jsonl");
    assert!(!l.contains("srv"), "{l}");
    assert_eq!(path_label(std::path::Path::new("out.jsonl")), "out.jsonl");
}
