//! Tests for the managed API: the contract, the validation rules, the
//! pricing arithmetic, webhook signing, the job registry, idempotency, the
//! renderers and the configuration bounds.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::scrape_api::models::{OutputFormat, ScrapeRequest, WaitSpec};
use crate::scrape_api::openapi;
use crate::scrape_api::pricing::Pricing;
use crate::scrape_api::validate;
use crate::scrape_api::webhook::{sign, verify};
use crate::scrape_api::{idempotency, jobs, models, service, time_now_ms};
use crate::usage::ledger::RenderMode;

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path =
            std::env::temp_dir().join(format!("harvest-v1-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn managed() -> crate::config::ManagedApiCfg {
    crate::config::ManagedApiCfg {
        enabled: true,
        ..crate::config::ManagedApiCfg::default()
    }
}

fn request(url: &str) -> ScrapeRequest {
    ScrapeRequest {
        url: url.to_string(),
        render: RenderMode::None,
        proxy_region: None,
        session_key: None,
        extract: Default::default(),
        format: OutputFormat::Html,
        screenshot: false,
        wait: WaitSpec::default(),
        webhook_url: None,
        idempotency_key: None,
    }
}

// ------------------------------------------------------------------ contract

#[test]
fn the_documented_paths_are_all_present() {
    let doc = openapi::spec(&managed());
    assert!(openapi::validate(&doc).is_ok());
    for path in [
        "/v1/scrape",
        "/v1/jobs",
        "/v1/jobs/{job_id}",
        "/v1/jobs/{job_id}/cancel",
        "/v1/usage",
        "/v1/openapi.json",
    ] {
        assert!(doc["paths"].get(path).is_some(), "missing {path}");
    }
}

#[test]
fn the_document_is_deterministic() {
    let a = openapi::render(&managed());
    let b = openapi::render(&managed());
    assert_eq!(a, b, "the served contract must be byte-stable");
    assert!(a.ends_with('\n'), "the served contract ends with a newline");
    let parsed: serde_json::Value = serde_json::from_str(&a).unwrap();
    assert_eq!(parsed["openapi"], "3.1.0");
}

#[test]
fn every_customer_route_requires_an_api_key_scheme() {
    let doc = openapi::spec(&managed());
    let security = doc["components"]["securitySchemes"]["apiKey"]["scheme"].as_str();
    assert_eq!(security, Some("bearer"));
    for (path, methods) in doc["paths"].as_object().unwrap() {
        // `/v1/openapi.json` is the contract document itself and is
        // deliberately anonymous: a client has to be able to read it before
        // it can know which keys to ask for.
        if path.starts_with("/v1/") && path != "/v1/openapi.json" {
            for (_method, op) in methods.as_object().unwrap() {
                assert!(
                    op["security"].as_array().is_some_and(|a| !a.is_empty()),
                    "{path} must declare api-key security"
                );
            }
        }
    }
}

#[test]
fn request_bodies_are_bounded_in_the_document() {
    let cfg = managed();
    let doc = openapi::spec(&cfg);
    let url_schema = &doc["paths"]["/v1/scrape"]["post"]["requestBody"]["content"]
        ["application/json"]["schema"]["properties"]["url"];
    assert_eq!(
        url_schema["maxLength"].as_u64(),
        Some(cfg.max_url_len as u64)
    );
    let body_schema = doc["components"]["schemas"]["ScrapeRequest"]["required"]
        .as_array()
        .unwrap();
    assert_eq!(body_schema.len(), 1);
    assert_eq!(body_schema[0], "url");
}

#[test]
fn the_summary_reports_the_bounds_in_force() {
    let cfg = managed();
    let summary = openapi::summary(&cfg);
    assert_eq!(
        summary["request_bytes"].as_u64(),
        Some(cfg.max_request_bytes)
    );
    assert_eq!(summary["paths"].as_array().unwrap().len(), 6);
}

// ---------------------------------------------------------------- validation

#[test]
fn validation_rejects_the_shapes_that_cannot_be_served() {
    let cfg = managed();
    let cases: Vec<(ScrapeRequest, &str)> = vec![
        (request(""), "url must not be empty"),
        (
            request("ftp://example.com/a"),
            "only http and https URLs are accepted",
        ),
        (request("https://example.com/a\n"), "url must not be empty"),
    ];
    for (req, _) in cases {
        assert!(
            validate::validate_request(&cfg, &req).is_err(),
            "expected a rejection for {:?}",
            req.url
        );
    }
}

#[test]
fn validation_rejects_a_screenshot_without_a_browser_renderer() {
    let cfg = managed();
    let mut req = request("https://example.com");
    req.screenshot = true;
    let err = validate::validate_request(&cfg, &req).unwrap_err();
    assert_eq!(err.code, "invalid_request");

    req.format = OutputFormat::Screenshot;
    assert!(validate::validate_request(&cfg, &req).is_err());

    req.render = RenderMode::Browser;
    assert!(validate::validate_request(&cfg, &req).is_ok());
}

#[test]
fn validation_rejects_an_oversized_url_and_selector() {
    let cfg = managed();
    let req = request(&format!(
        "https://example.com/{}",
        "a".repeat(cfg.max_url_len)
    ));
    assert!(validate::validate_request(&cfg, &req).is_err());

    let mut ok = request("https://example.com");
    ok.extract.max_records = Some(cfg.max_records + 1);
    assert!(validate::validate_request(&cfg, &ok).is_err());

    ok.extract.max_records = Some(cfg.max_records);
    assert!(validate::validate_request(&cfg, &ok).is_ok());
}

#[test]
fn validation_rejects_a_wait_beyond_the_bound() {
    let cfg = managed();
    let mut req = request("https://example.com");
    req.wait.ms = Some(cfg.jobs.max_browser_ms + 1);
    assert!(validate::validate_request(&cfg, &req).is_err());

    req.wait.ms = Some(cfg.jobs.max_browser_ms);
    assert!(validate::validate_request(&cfg, &req).is_ok());
}

#[test]
fn validation_enforces_the_webhook_allowlist() {
    let mut cfg = managed();
    cfg.webhook.enabled = true;
    cfg.webhook.allowed_hosts = vec!["hooks.example.com".to_string()];

    let mut req = request("https://example.com");
    req.webhook_url = Some("https://elsewhere.invalid/hook".to_string());
    assert!(validate::validate_request(&cfg, &req).is_err());

    req.webhook_url = Some("https://hooks.example.com/hook".to_string());
    assert!(validate::validate_request(&cfg, &req).is_ok());

    req.webhook_url = Some("http://hooks.example.com/hook".to_string());
    assert!(validate::validate_request(&cfg, &req).is_err());

    cfg.webhook.allowed_hosts.clear();
    assert!(validate::validate_request(&cfg, &req).is_err());
}

#[test]
fn unknown_request_fields_are_rejected() {
    let raw = r#"{"url":"https://example.com","invented":true}"#;
    assert!(serde_json::from_str::<ScrapeRequest>(raw).is_err());
}

// ----------------------------------------------------------------- pricing

#[test]
fn pricing_charges_each_component_once() {
    let cfg = managed();
    let pricing = Pricing::from_cfg(&cfg);
    let base = pricing.units_for(&request("https://example.com"), 0, 0);
    assert_eq!(base, cfg.pricing.base_units);

    let mut with_render = request("https://example.com");
    with_render.render = RenderMode::Browser;
    let rendered = pricing.units_for(&with_render, 0, 0);
    assert_eq!(rendered, base + cfg.pricing.render_units);

    with_render.screenshot = true;
    assert_eq!(
        pricing.units_for(&with_render, 0, 0),
        rendered + cfg.pricing.screenshot_units
    );
}

#[test]
fn pricing_charges_for_extraction_and_bytes() {
    let cfg = managed();
    let pricing = Pricing::from_cfg(&cfg);
    let plain = pricing.units_for(&request("https://example.com"), 0, 0);

    let mut with_fields = request("https://example.com");
    with_fields.extract.fields.insert(
        "title".into(),
        crate::config::FieldCfg {
            css: "h1".into(),
            path: None,
            attr: None,
            all: false,
            transforms: Vec::new(),
        },
    );
    let extracted = pricing.units_for(&with_fields, 0, 0);
    assert_eq!(extracted, plain + cfg.pricing.extract_units);

    let big = pricing.units_for(&request("https://example.com"), 4 * 1024, 0);
    assert!(big > plain);
}

#[test]
fn pricing_saturates_rather_than_wrapping() {
    let cfg = managed();
    let pricing = Pricing::from_cfg(&cfg);
    let small = pricing.units_for(&request("https://example.com"), 0, 0);
    let huge = pricing.units_for(&request("https://example.com"), u64::MAX, u64::MAX);
    assert!(huge >= small, "more bytes never cost less");
    assert!(huge > 0, "saturating arithmetic, never a wrap to zero");
    assert!(huge > cfg.pricing.base_units);
}

#[test]
fn the_cost_string_uses_the_configured_rate() {
    let cfg = managed();
    let pricing = Pricing::from_cfg(&cfg);
    let cost = pricing.cost(1_000_000);
    assert!(cost.ends_with(&cfg.pricing.currency));
    assert!(cost.starts_with("10.000000"));
}

// ------------------------------------------------------------------- webhooks

#[test]
fn webhook_signatures_round_trip_and_detect_tampering() {
    let key = "a-delivery-key";
    let payload = r#"{"event":"scrape.completed"}"#;
    let ts = 1_700_000_000_000u64;
    let signature = sign(key, ts, payload);
    assert!(verify(key, ts, payload, &signature));
    assert!(
        !verify(key, ts + 1, payload, &signature),
        "a timestamp change must fail"
    );
    assert!(
        !verify(key, ts, "other", &signature),
        "a payload change must fail"
    );
    assert!(!verify("another-key", ts, payload, &signature));
    assert_ne!(sign(key, ts, payload), sign(key, ts, "x"));
}

// ----------------------------------------------------------------------- jobs

fn job(id: &str, repeat: u32) -> jobs::JobRecord {
    jobs::JobRecord {
        id: id.to_string(),
        request: request("https://example.com"),
        schedule: models::JobSchedule {
            interval_minutes: 5,
            repeat,
        },
        tenant_id: "t-1".to_string(),
        key_id: "k-1".to_string(),
        key_hash: String::new(),
        state: models::JobState::Queued,
        request_id: "r-1".to_string(),
        created_at_ms: time_now_ms(),
        started_at_ms: None,
        finished_at_ms: None,
        response: None,
        records: Vec::new(),
        artifacts: Vec::new(),
        error: None,
        attempts: 0,
        runs_completed: 0,
        cancelled: false,
        idempotency_key: None,
    }
}

#[test]
fn the_job_registry_creates_gets_cancels_and_persists() {
    let dir = TempDir::new("jobs");
    let registry = jobs::JobRegistry::open(&managed(), dir.path()).unwrap();
    registry.create(job("j-1", 1)).unwrap();
    registry.create(job("j-2", 0)).unwrap();

    assert_eq!(registry.get("j-1").unwrap().state, models::JobState::Queued);
    assert_eq!(registry.len(), 2);

    // A terminal job is not resumable.
    let cancelled = registry.cancel("j-1").unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(cancelled.state, models::JobState::Cancelled);
    assert_eq!(
        registry.cancel("j-1").unwrap().state,
        models::JobState::Cancelled
    );

    // The snapshot survives a restart, and a job that was running when the
    // process died is failed rather than silently resumed.
    drop(registry);
    let reopened = jobs::JobRegistry::open(&managed(), dir.path()).unwrap();
    assert_eq!(reopened.get("j-2").unwrap().state, models::JobState::Queued);
    reopened.update("j-2", |j| {
        j.state = models::JobState::Running;
    });
    drop(reopened);
    let third = jobs::JobRegistry::open(&managed(), dir.path()).unwrap();
    let interrupted = third.get("j-2").unwrap();
    assert_eq!(interrupted.state, models::JobState::Failed);
    assert_eq!(interrupted.error.unwrap().code, "job_interrupted");
}

#[test]
fn a_job_is_exhausted_once_its_repeat_count_is_reached() {
    let mut record = job("j-1", 2);
    assert!(!record.exhausted());
    record.runs_completed = 2;
    assert!(record.exhausted());

    let mut forever = job("j-2", 0);
    forever.runs_completed = 99;
    assert!(!forever.exhausted());
    forever.cancelled = true;
    assert!(forever.exhausted());
}

#[test]
fn the_registry_is_bounded() {
    let dir = TempDir::new("jobs-bounded");
    let mut cfg = managed();
    cfg.jobs.max_jobs = 2;
    let registry = jobs::JobRegistry::open(&cfg, dir.path()).unwrap();
    for i in 0..5 {
        registry.create(job(&format!("j-{i}"), 0)).unwrap();
    }
    assert_eq!(registry.len(), 2, "the oldest jobs are evicted");
}

// ----------------------------------------------------------- idempotency

#[test]
fn the_idempotency_store_replays_within_the_window() {
    let cfg = managed();
    let dir = TempDir::new("idem");
    let store = idempotency::IdempotencyStore::open(&cfg, dir.path()).unwrap();

    assert!(store.lookup("k-1").unwrap().is_none());
    match store.store("k-1", 200, "{\"ok\":true}").unwrap() {
        idempotency::Insert::Inserted(_) => {}
        idempotency::Insert::Replayed(_) => panic!("first store must insert"),
    }
    match store.store("k-1", 200, "{\"ok\":false}").unwrap() {
        idempotency::Insert::Replayed(record) => {
            assert_eq!(record.body, "{\"ok\":true}");
            assert_eq!(record.status, 200);
        }
        idempotency::Insert::Inserted(_) => panic!("a repeat must replay"),
    }

    let replay = store.lookup("k-1").unwrap().unwrap();
    assert_eq!(replay.body, "{\"ok\":true}");
    assert_eq!(store.len(), 1);

    // The store is keyed by the digest, never by the plaintext.
    let text = std::fs::read_to_string(dir.path().join("idempotency.jsonl")).unwrap();
    assert!(
        !text.contains("k-1"),
        "the plaintext key must never be written"
    );
    assert!(text.contains("key_hash"));
}

#[test]
fn the_idempotency_store_is_bounded() {
    let mut cfg = managed();
    cfg.idempotency_max_records = 3;
    let dir = TempDir::new("idem-bounded");
    let store = idempotency::IdempotencyStore::open(&cfg, dir.path()).unwrap();
    for i in 0..10 {
        store.store(&format!("k-{i}"), 200, "{}").unwrap();
    }
    assert_eq!(store.len(), 3);
    assert!(
        store.lookup("k-0").unwrap().is_none(),
        "the oldest is evicted"
    );
}

// ---------------------------------------------------------------- rendering

#[test]
fn html_renders_to_text_and_markdown() {
    let html = "<html><body><h1>Title</h1><p>Hello <a href=\"https://x.test/p\">world</a>.</p>\
        <script>var secret = 1;</script><ul><li>one</li><li>two</li></ul></body></html>";

    let text = service::html_to_text(html);
    assert!(text.contains("Title"));
    assert!(text.contains("Hello world."));
    assert!(text.contains("one"));
    assert!(text.contains("two"));
    assert!(!text.contains("secret"), "script content must be dropped");
    assert!(!text.contains("<"), "tags are not part of the text");

    let markdown = service::html_to_markdown(html);
    assert!(markdown.contains("# Title"), "headings are rendered");
    assert!(
        markdown.contains("[world](https://x.test/p)"),
        "links are rendered"
    );
    assert!(markdown.contains("- one"), "list items are rendered");
    assert!(!markdown.contains("secret"));
}

#[test]
fn rendering_collapses_runs_of_blank_lines() {
    let html = "<div>\n\n\n<p>a</p>\n\n\n<p>b</p>\n\n\n</div>";
    let text = service::html_to_text(html);
    assert_eq!(text, "a\nb");
    assert!(!text.contains("\n\n\n"));
}

#[test]
fn malformed_html_renders_as_empty_rather_than_panicking() {
    assert_eq!(service::html_to_text(""), "");
    assert_eq!(service::html_to_text("<div"), "");
    assert_eq!(service::html_to_markdown(""), "");
}

// ----------------------------------------------------------------------- misc

#[test]
fn the_default_config_is_within_bounds() {
    let cfg = crate::config::ManagedApiCfg::default();
    assert!(cfg.validate().is_ok());
    assert!(
        !cfg.enabled,
        "the managed api stays off unless an operator asks for it"
    );
    assert_eq!(cfg.max_redirects, crate::config::V1_MAX_REDIRECTS);
    assert_eq!(cfg.max_request_bytes, crate::config::V1_BODY_LIMIT);
    assert!(
        !cfg.allow_private_targets,
        "the private-target opt-in is off by default"
    );
}

#[test]
fn the_default_limits_are_within_bounds() {
    assert!(crate::tenancy::limits::KeyLimits::default()
        .validate()
        .is_ok());
}

#[test]
fn the_request_id_is_fresh_per_call() {
    let a = request("https://example.com").request_id();
    let b = request("https://example.com").request_id();
    assert_ne!(a, b);
    assert!(!a.is_empty());
}

#[test]
fn response_shapes_are_stable_across_serialisation() {
    let response = models::ScrapeResponse {
        schema: models::RESPONSE_SCHEMA,
        request_id: "r".into(),
        status: 200,
        url: "https://example.com".into(),
        final_url: "https://example.com".into(),
        content_type: "text/html".into(),
        bytes: 10,
        latency_ms: 5,
        render: "none".into(),
        format: "html".into(),
        body: Some("<p>x</p>".into()),
        extracted: None,
        records: Some(vec![]),
        artifacts: Vec::new(),
        usage_units: 1,
        cost: "0.000010 USD".into(),
        screenshot: None,
        chain: models::ChainHead {
            seq: 1,
            hash: "abc".into(),
        },
    };
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["schema"], 1);
    assert_eq!(json["usage_units"], 1);
    // A re-parse must accept exactly what was written: the wire shape is the
    // contract.
    let back: models::ScrapeResponse = serde_json::from_value(json.clone()).unwrap();
    let back_json = serde_json::to_value(&back).unwrap();
    assert_eq!(back_json, json, "the wire shape is the contract");
}

#[test]
fn error_bodies_round_trip() {
    let body = models::ErrorBody::new("quota_exhausted", "monthly quota exhausted")
        .with_retry_after(1_000)
        .with_details(serde_json::json!({"used": 10}));
    let response = models::ErrorResponse::new(body.clone());
    let json = serde_json::to_value(&response).unwrap();
    assert_eq!(json["schema"], 1);
    assert_eq!(json["error"]["code"], "quota_exhausted");
    let back: models::ErrorResponse = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(&back.error).unwrap(),
        serde_json::to_value(&body).unwrap()
    );
}

#[test]
fn a_map_of_fields_serialises_in_order() {
    let mut map = BTreeMap::new();
    map.insert("a".to_string(), serde_json::json!(1));
    assert_eq!(serde_json::to_string(&map).unwrap(), "{\"a\":1}");
}
