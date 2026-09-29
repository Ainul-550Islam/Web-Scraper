use super::*;

#[test]
fn engine_layer_counters_render_and_serialize() {
    let m = Metrics::default();
    m.job_completed();
    m.job_failed();
    m.queue_rejected();
    m.breaker_rejected();
    m.retry_budget_denied();
    let txt = m.render_prometheus();
    for needle in [
        "harvest_jobs_completed 1",
        "harvest_jobs_failed 1",
        "harvest_queue_rejected 1",
        "harvest_breaker_rejected 1",
        "harvest_retry_budget_denied 1",
    ] {
        assert!(txt.contains(needle), "{needle} missing");
    }
    let j = m.display_json();
    assert_eq!(j["jobs_completed"], 1);
    assert_eq!(j["retry_budget_denied"], 1);
}

#[test]
fn pipeline_counters_in_both_expositions() {
    let m = Metrics::default();
    m.record_quarantined();
    for _ in 0..3 {
        m.schema_drift();
    }
    let txt = m.render_prometheus();
    assert!(txt.contains("harvest_records_quarantined 1"), "{txt}");
    assert!(txt.contains("harvest_schema_drifts 3"), "{txt}");
    let j = m.display_json();
    assert_eq!(j["records_quarantined"], 1);
    assert_eq!(j["schema_drifts"], 3);
}

#[test]
fn ops_counters_are_accounted_and_rendered() {
    let m = Metrics::default();
    m.pipeline_failures(3);
    m.note_request_ms(12);
    m.note_request_ms(30);
    m.note_storage_write(7);
    m.note_api_request(200, 4);
    m.note_api_request(401, 1);
    m.note_api_request(500, 9);
    m.note_api_request(404, 2);
    m.note_alert_fired(2);
    m.note_alert_notified();
    let j = m.display_json();
    assert_eq!(j["pipeline_failures"], 3);
    assert_eq!(j["request_ms_total"], 42);
    assert_eq!(j["storage_writes"], 1);
    assert_eq!(j["api_requests_2xx"], 1);
    assert_eq!(j["api_requests_4xx"], 2);
    assert_eq!(j["api_requests_5xx"], 1);
    assert_eq!(j["api_requests_total"], 4);
    assert_eq!(j["alerts_fired"], 2);
    assert_eq!(j["alerts_notified"], 1);
    let txt = m.render_prometheus();
    assert!(txt.contains("harvest_pipeline_failures 3"), "{txt}");
    assert!(
        txt.contains("harvest_api_requests_total{class=\"4xx\"} 2"),
        "{txt}"
    );
    assert!(txt.contains("harvest_api_request_ms_total 16"), "{txt}");
    // Bounded label set only: the sole label class is `class` and it
    // appears on exactly the three status-class series.
    assert_eq!(txt.matches("class=").count(), 3, "label sprawl");
}

#[test]
fn prometheus_and_json_keys_stay_in_lockstep() {
    // Dashboards scrape text, /stats serves JSON: a counter present in
    // one and not the other is an outage waiting to happen. The render
    // rows are the source of truth; every `harvest_x` there must also
    // appear as `x` in display_json (and vice versa, modulo none).
    let m = Metrics::default();
    let txt = m.render_prometheus();
    let j = m.display_json();
    let mut n = 0usize;
    for line in txt.lines() {
        if let Some(name) = line.strip_prefix("# HELP harvest_") {
            let name = name.split(' ').next().unwrap_or_default();
            n += 1;
            assert!(j.get(name).is_some(), "/stats misses {name}");
        }
    }
    assert!(n >= 20, "sanity: {n} counters is too few");
}

#[test]
fn per_target_family_and_cache_hits_render() {
    let m = Metrics::default();
    m.note_target_record("shop");
    m.note_target_record("shop");
    m.note_target_record("blog");
    m.robot_fetched();
    m.robot_cache_hit();
    m.robot_cache_hit();
    let txt = m.render_prometheus();
    assert!(
        txt.contains("harvest_target_records_written{target=\"blog\"} 1"),
        "{txt}"
    );
    assert!(
        txt.contains("harvest_target_records_written{target=\"shop\"} 2"),
        "{txt}"
    );
    assert!(txt.contains("harvest_robots_cache_hits 2"), "{txt}");
    let j = m.display_json();
    assert_eq!(j["target_records_written"]["shop"], 2);
    assert_eq!(j["target_records_written"]["blog"], 1);
    assert_eq!(j["robots_cache_hits"], 2);
    // empty map renders no family rows (no all-zero noise)
    let empty = Metrics::default();
    assert!(!empty
        .render_prometheus()
        .contains("harvest_target_records_written"));
}

#[test]
fn counters_are_thread_safe_additive() {
    use std::sync::Arc;
    let m = Arc::new(Metrics::default());
    let hs: Vec<_> = (0..4)
        .map(|_| {
            let m = m.clone();
            std::thread::spawn(move || {
                for _ in 0..250 {
                    m.request_started();
                    m.page_fetched();
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(
        m.requests_total.load(std::sync::atomic::Ordering::Relaxed),
        1000
    );
    assert_eq!(
        m.pages_fetched.load(std::sync::atomic::Ordering::Relaxed),
        1000
    );
}

#[test]
fn prometheus_render_contains_metrics() {
    let m = Metrics::default();
    m.request_started();
    m.request_started();
    m.record_written();
    let txt = m.render_prometheus();
    assert!(txt.contains("harvest_requests_total 2"), "{txt}");
    assert!(txt.contains("harvest_records_written 1"));
    assert!(txt.contains("# TYPE harvest_requests_total counter"));
}
