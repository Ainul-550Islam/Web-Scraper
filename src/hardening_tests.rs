//! Review regressions: exercise real boundaries, not just compilation.
use crate::config::{Config, SinkCfg};
use crate::export::{ExportFormat, ExportOpts};
use crate::sink::Record;
#[cfg(feature = "postgres")]
use crate::sink::Sink;
use serde_json::{json, Map};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct Temp(PathBuf);
impl Temp {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "harvest-review-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn rec() -> Record {
    Record {
        run: "review-run".into(),
        target: "review-target".into(),
        url: "https://example.test/a".into(),
        depth: 1,
        ts: 1700000000,
        fingerprint: "review-fp".into(),
        egress: "direct".into(),
        fields: Map::from_iter([
            ("title".into(), json!("বাংলা title")),
            ("count".into(), json!(42)),
        ]),
    }
}
fn input(dir: &Path) -> PathBuf {
    let path = dir.join("input.jsonl");
    std::fs::write(
        &path,
        format!("{}\n", serde_json::to_string(&rec()).unwrap()),
    )
    .unwrap();
    path
}
fn opts(input: PathBuf, output: PathBuf) -> ExportOpts {
    ExportOpts {
        inputs: vec![input],
        out_dir: output,
        split_target: false,
        history: None,
        key: None,
        max_part_bytes: 0,
        resume: false,
        formats: vec![ExportFormat::Csv, ExportFormat::Jsonl],
        dataset: None,
        pdf_sample_rows: 12,
    }
}
fn spec() -> crate::export_job::ExportJobSpec {
    crate::export_job::ExportJobSpec {
        formats: vec![ExportFormat::Csv, ExportFormat::Jsonl],
        dataset: None,
        bundle: false,
        verify: true,
        max_records: 10,
        max_export_bytes: 1024 * 1024,
    }
}
#[test]
fn audit_export_budget_fails_before_completion_publication() {
    let t = Temp::new("write-cap");
    let options = opts(input(&t.0), t.0.join("delivery"));
    let error = crate::export::run_bounded(&options, 1).unwrap_err();
    assert!(format!("{error:#}").contains("budget"));
    assert!(!options.out_dir.join("manifest.json").exists());
}
#[test]
fn audit_export_never_overwrites_a_source_in_its_delivery() {
    let t = Temp::new("collision");
    let source = input(&t.0);
    let before = std::fs::read(&source).unwrap();
    let error = crate::export::run(&opts(source.clone(), t.0.clone())).unwrap_err();
    assert!(format!("{error:#}").contains("outside"));
    assert_eq!(std::fs::read(source).unwrap(), before);
}
#[test]
fn audit_zip_only_contains_real_record_payloads() {
    let t = Temp::new("zip-only");
    let mut options = opts(input(&t.0), t.0.join("delivery"));
    options.formats = vec![ExportFormat::Zip];
    let result = crate::export::run(&options).unwrap();
    assert!(result
        .manifest
        .files
        .iter()
        .any(|f| f.name.ends_with("jsonl")));
    assert!(result.bundle.is_some());
    assert!(crate::export::verify(&options.out_dir, None).unwrap().ok);
}
#[test]
fn audit_export_manifest_contains_no_host_source_paths() {
    let t = Temp::new("source-label");
    let options = opts(input(&t.0), t.0.join("delivery"));
    let report = crate::export::run(&options).unwrap();
    assert_eq!(report.manifest.source_files, vec!["input.jsonl"]);
    assert!(
        !std::fs::read_to_string(options.out_dir.join("manifest.json"))
            .unwrap()
            .contains(t.0.to_str().unwrap())
    );
}
#[test]
fn audit_chunk_fanout_has_a_finite_delivery_file_cap() {
    let t = Temp::new("parts");
    let source = t.0.join("input.jsonl");
    let body=(0..300).map(|n| format!("{}\n",json!({"run":"r","target":"t","url":format!("https://example.test/{n}"),"fields":{"title":"x"}}))).collect::<String>();
    std::fs::write(&source, body).unwrap();
    let mut options = opts(source, t.0.join("delivery"));
    options.max_part_bytes = 1;
    let error = crate::export::run(&options).unwrap_err();
    assert!(format!("{error:#}").contains("file"));
    assert!(!options.out_dir.join("manifest.json").exists());
}
#[cfg(unix)]
#[test]
fn audit_output_hardlink_is_refused_without_truncating_original() {
    let t = Temp::new("hardlink");
    let original = t.0.join("private");
    let output = t.0.join("output");
    std::fs::write(&original, b"keep me").unwrap();
    std::fs::hard_link(&original, &output).unwrap();
    assert!(crate::export_io::create(&output).is_err());
    assert_eq!(std::fs::read(original).unwrap(), b"keep me");
}
#[cfg(unix)]
#[test]
fn audit_dataset_sink_rejects_a_symlinked_output_root() {
    let t = Temp::new("dataset-output-link");
    let real = t.0.join("real");
    std::fs::create_dir_all(&real).unwrap();
    let alias = t.0.join("alias");
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let cfg = SinkCfg::Dataset {
        dir: alias,
        partition: "day".into(),
    };
    assert!(crate::sink::build(&cfg).is_err());
    assert_eq!(std::fs::read_dir(real).unwrap().count(), 0);
}

#[test]
fn audit_csv_source_can_export_without_pretending_it_is_jsonl() {
    let t = Temp::new("csv-source");
    let source = t.0.join("data.csv");
    let cfg = Config {
        output: serde_json::from_value(json!({"kind":"csv","path":source})).unwrap(),
        ..Default::default()
    };
    let (sink, _) = crate::sink::build(&cfg.output).unwrap();
    sink.write(&rec()).unwrap();
    sink.finish().unwrap();
    let (result, _) = crate::export_job::run(&spec(), &cfg, &t.0.join("delivery")).unwrap();
    assert_eq!(result.records, 1);
    assert!(matches!(
        result.verification,
        crate::download::VerificationState::Ok { .. }
    ));
}
#[cfg(feature = "sqlite")]
#[test]
fn audit_sqlite_wal_is_flushed_and_its_source_is_exportable() {
    let t = Temp::new("sqlite-source");
    let source = t.0.join("data.sqlite");
    let cfg = Config {
        output: serde_json::from_value(json!({"kind":"sqlite","path":source})).unwrap(),
        ..Default::default()
    };
    let (sink, _) = crate::sink::build(&cfg.output).unwrap();
    sink.write(&rec()).unwrap();
    sink.finish().unwrap();
    let snapshot = t.0.join("snapshot.sqlite");
    std::fs::copy(&source, &snapshot).unwrap();
    let conn = rusqlite::Connection::open(&snapshot).unwrap();
    let count: i64 = conn
        .query_row("select count(*) from records", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let (result, _) = crate::export_job::run(&spec(), &cfg, &t.0.join("delivery")).unwrap();
    assert_eq!(result.records, 1);
    assert!(matches!(
        result.verification,
        crate::download::VerificationState::Ok { .. }
    ));
}
#[cfg(all(feature = "sqlite", unix))]
#[test]
fn audit_sqlite_rejects_symlink_hardlink_and_sidecar_aliases() {
    use std::os::unix::fs::symlink;

    let t = Temp::new("sqlite-path-safety");
    let original = t.0.join("original.sqlite");
    let cfg = Config {
        output: serde_json::from_value(json!({"kind":"sqlite","path":original})).unwrap(),
        ..Default::default()
    };
    let (sink, _) = crate::sink::build(&cfg.output).unwrap();
    sink.write(&rec()).unwrap();
    sink.finish().unwrap();
    drop(sink);

    let original = match &cfg.output {
        SinkCfg::Sqlite { path } => path.clone(),
        _ => unreachable!(),
    };
    let link = t.0.join("linked.sqlite");
    symlink(&original, &link).unwrap();
    let link_cfg = Config {
        output: SinkCfg::Sqlite { path: link },
        ..Default::default()
    };
    assert!(crate::sink::build(&link_cfg.output).is_err());
    let visit_link = crate::source_io::visit(
        &link_cfg,
        &[match &link_cfg.output {
            SinkCfg::Sqlite { path } => path.clone(),
            _ => unreachable!(),
        }],
        1,
        |_| Ok(()),
    );
    assert!(visit_link.is_err());

    let hard_link = t.0.join("hard-linked.sqlite");
    std::fs::hard_link(&original, &hard_link).unwrap();
    let hard_cfg = Config {
        output: SinkCfg::Sqlite {
            path: hard_link.clone(),
        },
        ..Default::default()
    };
    assert!(crate::sink::build(&hard_cfg.output).is_err());

    let sidecar_db = t.0.join("sidecar.sqlite");
    std::fs::copy(&original, &sidecar_db).unwrap();
    let mut wal_name = sidecar_db.as_os_str().to_os_string();
    wal_name.push("-wal");
    symlink(&original, PathBuf::from(wal_name)).unwrap();
    let sidecar_cfg = Config {
        output: SinkCfg::Sqlite {
            path: sidecar_db.clone(),
        },
        ..Default::default()
    };
    assert!(crate::sink::build(&sidecar_cfg.output).is_err());
    assert!(crate::source_io::visit(&sidecar_cfg, &[sidecar_db], 1, |_| Ok(())).is_err());
}

#[test]
fn audit_dataset_export_discovers_day_partitions() {
    let t = Temp::new("dataset");
    let root = t.0.join("dataset");
    let part = root.join("dt=2026-10-02");
    std::fs::create_dir_all(&part).unwrap();
    input(&part);
    let generated = root.join(".harvest-control/run-snapshots/job-example");
    std::fs::create_dir_all(&generated).unwrap();
    input(&generated);
    let cfg = Config {
        output: serde_json::from_value(json!({"kind":"dataset","dir":root,"partition":"day"}))
            .unwrap(),
        ..Default::default()
    };
    let sources = crate::export_job::source_files(&cfg).unwrap();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0], part.join("input.jsonl"));
    let (result, _) = crate::export_job::run(&spec(), &cfg, &t.0.join("delivery")).unwrap();
    assert_eq!(result.records, 1);
}
#[test]
fn audit_dataset_parts_are_not_silently_truncated() {
    let t = Temp::new("dataset-count");
    for n in 0..65 {
        std::fs::write(t.0.join(format!("part-{n}.jsonl")), "{}\n").unwrap();
    }
    assert!(crate::source_io::dataset_files(&t.0)
        .unwrap_err()
        .to_string()
        .contains("64"));
}
#[cfg(unix)]
#[test]
fn audit_dataset_symlink_cannot_escape_the_approved_source() {
    let t = Temp::new("dataset-link");
    let root = t.0.join("dataset");
    std::fs::create_dir_all(&root).unwrap();
    std::os::unix::fs::symlink(input(&t.0), root.join("leak.jsonl")).unwrap();
    assert!(crate::source_io::dataset_files(&root).is_err());
}
#[test]
fn audit_sensitive_url_queries_and_event_arrays_are_redacted() {
    let mut value = json!({"events":[{"url":"https://example.test/path?token=topsecret&page=2"},{"authorization":"Bearer topsecret"},{"reason":"request https://alice:pw@example.test/private failed"}]});
    crate::config::redact_observation(&mut value);
    let text = value.to_string();
    assert!(!text.contains("topsecret"));
    assert!(!text.contains("alice"));
    assert!(!text.contains("pw"));
    assert!(!text.contains("private"));
    assert!(text.contains("page=2"));
}
#[test]
fn audit_nested_config_typos_are_not_silently_ignored() {
    for wrong in [
        json!({"schedule":{"every_sec":20}}),
        json!({"alerts":{"min_record":5}}),
        json!({"scheduling":{"max_queue_dept":4}}),
        json!({"output":{"kind":"jsonl","path":"x","typo":true}}),
    ] {
        assert!(serde_json::from_value::<Config>(wrong).is_err());
    }
}
#[test]
fn audit_remote_redis_requires_verified_tls_and_forbids_insecure() {
    assert!(crate::config::validate_redis_url("redis://127.0.0.1:6379/0").is_ok());
    assert!(crate::config::validate_redis_url("redis://localhost:6379/0").is_ok());
    assert!(crate::config::validate_redis_url("redis://[::1]:6379/0").is_ok());
    assert!(crate::config::validate_redis_url("redis://db.example.test/0").is_err());
    assert!(crate::config::validate_redis_url("rediss://db.example.test/0").is_ok());
    assert!(crate::config::validate_redis_url("rediss://db.example.test/0#insecure").is_err());
}
#[test]
fn audit_robots_octets_unicode_and_reserved_slashes_are_distinguished() {
    let rules = crate::robots::Robots::parse(
        "User-agent: *\nDisallow: /private/A\nDisallow: /বাংলা\nDisallow: /x%2Fy\n",
        "harvest",
    );
    assert!(!rules.allows("/private/%41"));
    assert!(!rules.allows("/private/A"));
    assert!(!rules.allows("/%E0%A6%AC%E0%A6%BE%E0%A6%82%E0%A6%B2%E0%A6%BE"));
    assert!(!rules.allows("/x%2fy"));
    assert!(rules.allows("/x/y"));
}
#[test]
fn audit_robots_unknown_directives_do_not_split_agent_groups() {
    let r = crate::robots::Robots::parse(
        "User-agent: harvest\nUnknown: ignored\nUser-agent: other\nDisallow: /private\n",
        "harvest",
    );
    assert!(!r.allows("/private"));
}
#[test]
fn audit_oversized_robots_policies_fail_closed() {
    let policy = format!("User-agent: *\n{}", "# x\n".repeat(140000));
    assert!(!crate::robots::Robots::parse(&policy, "harvest").allows("/"));
}
#[test]
fn audit_css_record_extraction_rejects_memory_amplification() {
    let rules = std::collections::BTreeMap::from([(
        "text".into(),
        serde_json::from_value(json!({"css":"div","all":true})).unwrap(),
    )]);
    let ex = crate::extract::Extractor::compile(&rules).unwrap();
    let body = format!("<div>{}</div>", "a".repeat(1024 * 1024 + 1));
    assert!(ex
        .extract_bounded(&scraper::Html::parse_document(&body))
        .is_err());
    assert!(crate::extract::guard_html(&"<br>".repeat(100001)).is_err());
}
#[test]
fn audit_json_extraction_is_bounded_before_cloning_large_fields() {
    let rules = std::collections::BTreeMap::from([(
        "data".into(),
        serde_json::from_value(json!({"path":"$.huge"})).unwrap(),
    )]);
    let ex = crate::extract::JsonExtractor::compile(&rules).unwrap();
    let doc = json!({"huge":"a".repeat(8*1024*1024+1)});
    assert!(ex.extract_bounded(&doc).is_err());
}
#[test]
fn audit_json_fingerprint_is_not_an_html_fingerprint() {
    assert_eq!(
        crate::extract::fingerprint_json("{\"a\":1,\"b\":2}").unwrap(),
        crate::extract::fingerprint_json("{\"b\":2,\"a\":1}").unwrap()
    );
    assert_ne!(
        crate::extract::fingerprint_json("{\"text\":\"<b>A</b>\"}").unwrap(),
        crate::extract::fingerprint_json("{\"text\":\"<i>A</i>\"}").unwrap()
    );
}
#[tokio::test]
async fn audit_waiting_for_storage_can_be_cancelled_without_the_holder_finishing() {
    let slot = crate::runtime::RuntimeSlot::default();
    let _holder = slot.output_guard(None).await.unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let waiting = slot.clone();
    let task =
        tokio::spawn(async move { waiting.output_guard(Some(stopped.as_ref())).await.is_err() });
    tokio::time::sleep(Duration::from_millis(20)).await;
    stop.store(true, Ordering::Relaxed);
    assert!(tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap());
}
#[tokio::test]
async fn audit_pacing_is_preserved_across_run_epochs() {
    let mut cfg = Config::default();
    cfg.default_policy.requests_per_sec = 1000.0;
    cfg.default_policy.burst = 100;
    cfg.default_policy.min_interval_ms = 200;
    let slot = crate::runtime::RuntimeSlot::default();
    let p = slot.shared_pacing(&cfg);
    let metrics = crate::metrics::Metrics::default();
    p.wait("example.test", &cfg, &metrics).await;
    p.begin_epoch();
    let start = Instant::now();
    slot.isolated()
        .shared_pacing(&cfg)
        .wait("example.test", &cfg, &metrics)
        .await;
    assert!(start.elapsed() >= Duration::from_millis(180));
}
#[test]
fn audit_long_retry_after_is_not_shortened_or_reset_by_a_new_run() {
    let cfg = Config::default();
    let p = crate::policy::Politeness::new(&cfg);
    assert_eq!(
        p.note_block("example.test", &cfg, Some(Duration::from_secs(7200))),
        Duration::from_secs(7200)
    );
    p.begin_epoch();
    assert!(p.is_abandoned("example.test", &cfg));
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_cancellation_interrupts_an_active_response_not_only_backoff() {
    let server = crate::testutil::MockServer::start(vec![(
        "/slow",
        crate::testutil::Route::Delay(
            Duration::from_secs(2),
            Box::new(crate::testutil::Route::ok("late")),
        ),
    )]);
    let stop = Arc::new(AtomicBool::new(false));
    let trigger = stop.clone();
    let cfg = Arc::new(Config {
        max_retries: 0,
        request_timeout_secs: 10,
        ..Default::default()
    });
    let fetcher = crate::fetcher::Fetcher::new(cfg, Arc::new(crate::proxy::ProxyPool::new(&[])))
        .with_cancellation(vec![stop], None);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(80)).await;
        trigger.store(true, Ordering::Relaxed);
    });
    let start = Instant::now();
    assert!(fetcher
        .get(
            url::Url::parse(&format!("{}/slow", server.base())).unwrap(),
            &crate::metrics::Metrics::default()
        )
        .await
        .is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
}
#[test]
fn audit_http_post_redirect_is_a_failure_and_the_worker_stops_before_finish_returns() {
    let server = crate::testutil::MockServer::start(vec![
        (
            "/sink",
            crate::testutil::Route::Redirect(303, "/success".into()),
        ),
        (
            "/success",
            crate::testutil::Route::ok("not an acknowledgement"),
        ),
    ]);
    let cfg: SinkCfg=serde_json::from_value(json!({"kind":"http","url":format!("{}/sink",server.base()),"timeout_secs":1,"batch_max":1})).unwrap();
    let (sink, _) = crate::sink::build(&cfg).unwrap();
    for _ in 0..10 {
        sink.write(&rec()).unwrap();
    }
    assert!(sink.finish().is_err());
    assert!(sink.write(&rec()).is_err());
}
#[test]
fn audit_cancelled_blocking_export_has_no_completion_marker() {
    let t = Temp::new("cancel-export");
    let options = opts(input(&t.0), t.0.join("delivery"));
    let _cancel = crate::work_cancel::begin(Arc::new(AtomicBool::new(true)));
    assert!(crate::export::run(&options).is_err());
    assert!(!options.out_dir.join("manifest.json").exists());
}
#[cfg(feature = "postgres")]
#[test]
fn audit_postgres_tls_connects_with_trusted_ca_and_rejects_wrong_hostname() {
    let Ok(dsn) = std::env::var("HARVEST_TEST_PG_TLS_DSN") else {
        return;
    };
    let sink = crate::pgsink::PgSink::open(dsn.clone()).unwrap();
    let mut record = rec();
    record.fingerprint = format!(
        "tls-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    sink.write(&record).unwrap();
    sink.finish().unwrap();
    assert_eq!(sink.rows_written.load(Ordering::Relaxed), 1);
    let bad = crate::pgsink::PgSink::open(dsn.replace("host=127.0.0.1", "host=127.0.0.2")).unwrap();
    bad.write(&rec()).unwrap();
    assert!(bad.finish().is_err());
    assert_eq!(bad.rows_written.load(Ordering::Relaxed), 0);
}
#[cfg(feature = "redis")]
#[tokio::test]
async fn audit_shared_redis_fractional_rate_spacing_and_stop_are_real() {
    if std::env::var("HARVEST_TEST_REDIS").as_deref() != Ok("1") {
        return;
    }
    let uri = std::env::var("HARVEST_TEST_REDIS_URL").unwrap();
    let prefix = format!(
        "auditpace{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut q = crate::dist::Queue::connect(&uri, &prefix).await.unwrap();
    q.wait_host("h", 2.5, None).await.unwrap();
    let start = Instant::now();
    q.clone().wait_host("h", 2.5, None).await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(350));
    let stop = Arc::new(AtomicBool::new(false));
    q.wait_host("slow", 1.0 / 3600.0, None).await.unwrap();
    let signal = stop.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        signal.store(true, Ordering::Relaxed);
    });
    assert!(q
        .wait_host("slow", 1.0 / 3600.0, Some(&stop))
        .await
        .is_err());
    let policy = crate::config::DomainPolicy {
        requests_per_sec: 1000.0,
        burst: 100,
        min_interval_ms: 0,
    };
    q.wait_policy("delay", &policy, None).await.unwrap();
    q.note_crawl_delay("delay", Duration::from_millis(200))
        .await
        .unwrap();
    let start = Instant::now();
    q.wait_policy("delay", &policy, None).await.unwrap();
    assert!(start.elapsed() >= Duration::from_millis(180));
    assert!(q.reserve_page("target", 1, 1).await.unwrap());
    assert!(!q.reserve_page("target", 1, 1).await.unwrap());
    q.note_block(
        "refused",
        &Config::default(),
        Some(Duration::from_secs(7200)),
    )
    .await
    .unwrap();
    assert!(q.wait_policy("refused", &policy, None).await.is_err());
    q.flush_prefix().await.unwrap();
}
#[cfg(feature = "redis")]
#[tokio::test]
async fn audit_redis_tls_and_hostname_validation() {
    let Ok(uri) = std::env::var("HARVEST_TEST_REDIS_TLS_URL") else {
        return;
    };
    let mut q = crate::dist::Queue::connect(&uri, "audit_tls")
        .await
        .unwrap();
    assert!(q.counts().await.unwrap().is_object());
    let bad = uri.replace("127.0.0.1", "127.0.0.2");
    assert!(crate::dist::Queue::connect(&bad, "audit_tls_bad")
        .await
        .is_err());
}

#[test]
fn audit_domain_policy_is_case_insensitive() {
    let mut cfg = Config::default();
    let policy = crate::config::DomainPolicy {
        requests_per_sec: 0.25,
        burst: 1,
        min_interval_ms: 1000,
    };
    cfg.domain_policies.insert("EXAMPLE.test.".into(), policy);
    assert_eq!(cfg.policy_for("example.test").requests_per_sec, 0.25);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_redirect_permission_and_unavailable_robots_are_enforced_before_get() {
    for (tag, robots, excluded) in [
        (
            "robots",
            crate::testutil::Route::ok("User-agent: *\nDisallow: /private\n"),
            false,
        ),
        (
            "scope",
            crate::testutil::Route::ok("User-agent: *\nDisallow:\n"),
            true,
        ),
        (
            "unavailable",
            crate::testutil::Route::status(503, "temporarily down"),
            false,
        ),
    ] {
        let server = crate::testutil::MockServer::start(vec![
            ("/robots.txt", robots),
            (
                "/go",
                crate::testutil::Route::Redirect(302, "/private".into()),
            ),
            (
                "/private",
                crate::testutil::Route::ok("<h1>must not reach</h1>"),
            ),
        ]);
        let fx = crate::testutil::TempFixture::new(&format!("audit-{tag}"), &server.base());
        let mut cfg = fx.config();
        cfg.targets[0].start_url = format!("{}/go", server.base());
        cfg.targets[0].follow_links = false;
        cfg.targets[0].include.clear();
        cfg.targets[0].exclude = if excluded {
            vec!["/private".into()]
        } else {
            vec![]
        };
        cfg.respect_robots = !excluded;
        cfg.max_retries = 0;
        cfg.default_policy.requests_per_sec = 1000.0;
        cfg.default_policy.burst = 100;
        cfg.default_policy.min_interval_ms = 0;
        let summary =
            crate::engine::run_with(Arc::new(cfg), crate::engine::RunCtx::default(), None)
                .await
                .unwrap();
        assert_eq!(summary.written, 0);
        assert!(!server.hits().iter().any(|p| p == "/private"));
        if tag == "unavailable" {
            assert!(!server.hits().iter().any(|p| p == "/go"));
        }
    }
}

#[cfg(feature = "redis")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn audit_distributed_json_and_expired_job_cleanup() {
    if std::env::var("HARVEST_TEST_REDIS").as_deref() != Ok("1") {
        return;
    }
    let uri = std::env::var("HARVEST_TEST_REDIS_URL").unwrap();
    let t = Temp::new("distributed-json");
    let server = crate::testutil::MockServer::start(vec![(
        "/api",
        crate::testutil::Route::Respond {
            status: 200,
            body: "{\"answer\":42}".into(),
            content_type: "application/json",
            headers: vec![],
        },
    )]);
    let cfg:Config=serde_json::from_value(json!({"name":"audit-json","respect_robots":false,"metrics_addr":"","default_policy":{"requests_per_sec":1000,"burst":100,"min_interval_ms":0},"targets":[{"id":"json","mode":"json","start_url":format!("{}/api",server.base()),"follow_links":false,"extract":{"answer":{"path":"$.answer"}}}],"output":{"kind":"jsonl","path":t.0.join("data.jsonl")}})).unwrap();
    let prefix = format!(
        "auditjson{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut q = crate::dist::Queue::connect(&uri, &prefix).await.unwrap();
    let job = crate::dist::Job {
        url: cfg.targets[0].start_url.clone(),
        depth: 0,
        target: 0,
        attempts: 0,
        expires_at: None,
    };
    q.push_job(&job).await.unwrap();
    let stats = crate::dist::worker(
        cfg.clone(),
        crate::dist::WorkerOpts {
            redis_url: uri.clone(),
            prefix: prefix.clone(),
            name: "audit-json-worker".into(),
            idle_secs: 1,
            queue_fail_limit: 2,
            stop: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(stats.written, 1);
    let body =
        std::fs::read_to_string(t.0.join(format!("data.p{}.jsonl", std::process::id()))).unwrap();
    let v: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(v["fields"]["answer"], 42);
    q.push_job(&crate::dist::Job {
        expires_at: Some(0),
        ..job
    })
    .await
    .unwrap();
    let stats = crate::dist::worker(
        cfg,
        crate::dist::WorkerOpts {
            redis_url: uri,
            prefix: prefix.clone(),
            name: "audit-expired-worker".into(),
            idle_secs: 1,
            queue_fail_limit: 2,
            stop: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(stats.dead, 1);
    let counts = q.counts().await.unwrap();
    assert_eq!(counts["inflight"], 0);
    assert_eq!(counts["jobs"], 0);
    q.flush_prefix().await.unwrap();
}
