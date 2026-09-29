use super::*;

const GOOD: &str = r#"{
    "targets": [{
        "id": "docs",
        "start_url": "https://example.com/index.html",
        "extract": { "title": { "css": "h1" } }
    }],
    "output": { "kind": "null" }
}"#;

/// Single-target config with one `f: css h1` field carrying `schema`.
fn t1(schema: SchemaCfg, transforms: Vec<TransformCfg>) -> Config {
    let mut t = TargetCfg {
        start_url: "http://example.com".into(),
        schema,
        ..Default::default()
    };
    t.extract.insert(
        "f".into(),
        FieldCfg {
            css: "h1".into(),
            path: None,
            attr: None,
            all: false,
            transforms,
        },
    );
    Config {
        targets: vec![t],
        ..Default::default()
    }
}

#[test]
fn schema_strict_requires_quarantine_path_at_validate() {
    let strict = SchemaCfg {
        enforce: "strict".into(),
        fields: Default::default(),
        quarantine_path: None,
    };
    let err = t1(strict, Vec::new()).validate().unwrap_err().to_string();
    assert!(err.contains("quarantine_path"), "{err}");
    // with the path set, strict validates
    let ok = SchemaCfg {
        enforce: "strict".into(),
        fields: Default::default(),
        quarantine_path: Some("q.jsonl".into()),
    };
    t1(ok, Vec::new())
        .validate()
        .expect("strict + path is complete");
}

#[test]
fn unknown_schema_type_and_enforce_word_are_rejected() {
    let bad_ty = SchemaCfg {
        enforce: "coerce".into(),
        fields: std::collections::BTreeMap::from([("f".to_string(), "currency".to_string())]),
        quarantine_path: None,
    };
    let err = t1(bad_ty, Vec::new()).validate().unwrap_err().to_string();
    assert!(err.contains("unknown schema type"), "{err}");

    let bad_word = SchemaCfg {
        enforce: "paranoid".into(),
        fields: Default::default(),
        quarantine_path: None,
    };
    let err = t1(bad_word, Vec::new()).validate().unwrap_err().to_string();
    assert!(err.contains("off | coerce | strict"), "{err}");
}

#[test]
fn transform_regexes_compile_during_validate() {
    let err = t1(
        SchemaCfg::default(),
        vec![TransformCfg::RegexExtract {
            pattern: "((broken".into(),
            group: 1,
        }],
    )
    .validate()
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("field `f`") && err.contains("regex_extract"),
        "{err}"
    );
}

#[test]
fn transform_semantic_knobs_are_checked_not_just_parsed() {
    let err = |tf: TransformCfg| {
        t1(SchemaCfg::default(), vec![tf])
            .validate()
            .unwrap_err()
            .to_string()
    };
    assert!(err(TransformCfg::MaskPii {
        strategy: MaskStrategy::Redact,
        kinds: vec!["iban".into()],
    })
    .contains("mask_pii kind"));
    assert!(err(TransformCfg::Units {
        to: "fathoms".into()
    })
    .contains("units.to"));
    assert!(err(TransformCfg::ParseDate {
        output: "excel-1900".into()
    })
    .contains("parse_date.output"));
    assert!(err(TransformCfg::Split {
        sep: String::new(),
        trim: false,
        max: 0,
    })
    .contains("split.sep"));
}

#[test]
fn transform_and_schema_json_shape_is_tagged_enums() {
    // The on-disk shape is the user contract; pin it.
    let cfg: FieldCfg = serde_json::from_str(
        r#"{ "css": "h1", "transforms": [
              { "op": "mask_pii", "strategy": "hash", "kinds": ["email", "card"] },
              { "op": "parse_date", "output": "unix" } ] }"#,
    )
    .unwrap();
    assert_eq!(cfg.transforms.len(), 2);
    assert_eq!(
        cfg.transforms[0],
        TransformCfg::MaskPii {
            strategy: MaskStrategy::Hash,
            kinds: vec!["email".into(), "card".into()]
        }
    );
    let back = serde_json::to_string(&cfg.transforms[1]).unwrap();
    assert!(
        back.contains(r#""op":"parse_date""#) && back.contains(r#""output":"unix""#),
        "{back}"
    );
    // unknown op must fail closed
    assert!(serde_json::from_str::<TransformCfg>(r#"{ "op": "titlecase_or_noop" }"#).is_err());
    // schema block defaults to off even when absent entirely
    let t: TargetCfg =
        serde_json::from_str(r#"{ "id": "a", "start_url": "http://x/", "extract": {} }"#).unwrap();
    assert_eq!(t.schema.enforce, "off");
    assert!(t.schema.fields.is_empty() && t.schema.quarantine_path.is_none());
}

#[test]
fn loads_with_defaults() {
    let mut cfg: Config = serde_json::from_str(GOOD).unwrap();
    cfg.validate().unwrap();
    assert_eq!(cfg.concurrency, 4);
    assert!(cfg.respect_robots);
    assert_eq!(cfg.max_pages, 500);
    assert_eq!(
        cfg.metrics_addr, "127.0.0.1:9678",
        "admin default must bind loopback"
    );
    assert_eq!(cfg.max_response_bytes, 16 * 1024 * 1024);
}

#[test]
fn rejects_browser_spoofing_ua() {
    let raw = GOOD.replace(
        "\"targets\"",
        "\"user_agent\": \"Mozilla/5.0 (Windows NT 10.0)\", \"targets\"",
    );
    let mut cfg: Config = serde_json::from_str(&raw).unwrap();
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("honest bot UA"), "{err}");
}

#[test]
fn schedule_footgun_rejected_and_hash_stable() {
    let raw = GOOD.replace(
        "\"output\"",
        "\"schedule\": { \"every_secs\": 0, \"runs\": 0 }, \"output\"",
    );
    let mut cfg: Config = serde_json::from_str(&raw).unwrap();
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("schedule footgun"), "{err}");

    let ok: Config = serde_json::from_str(GOOD).unwrap();
    assert_eq!(ok.hash().len(), 64);
    assert_eq!(ok.hash(), ok.hash(), "hash must be deterministic");
    let mut changed = ok.clone();
    changed.name = "different".into();
    assert_ne!(ok.hash(), changed.hash(), "hash must change with config");
}

#[test]
fn rejects_empty_targets_and_bad_selector() {
    let mut empty: Config = serde_json::from_str(r#"{"targets": []}"#).unwrap();
    assert!(empty.validate().is_err());

    let raw = GOOD.replace("\"h1\"", "\"##broken##\"");
    let mut cfg: Config = serde_json::from_str(&raw).unwrap();
    let err = cfg.validate().unwrap_err();
    assert!(err.to_string().contains("invalid CSS selector"), "{err}");
}

// ---- security & config-safety (prompt-2 hardening) --------------------

#[test]
fn top_level_keys_match_the_serialization() {
    // Typo detection (migrate::check_unknown_keys) is only as good as
    // this list; a field added without updating it breaks the test.
    let v = serde_json::to_value(Config::default()).unwrap();
    let keys: std::collections::BTreeSet<&str> =
        v.as_object().unwrap().keys().map(|s| s.as_str()).collect();
    let known: std::collections::BTreeSet<&str> = Config::TOP_LEVEL_KEYS.iter().copied().collect();
    assert_eq!(keys, known, "TOP_LEVEL_KEYS drifted from the struct");
}

#[test]
fn load_migrates_v1_and_surfaces_warnings() {
    let dir = tempdir_like("migrate-load");
    let p = dir.join("cfg.json");
    // a genuine v1 file: no config_version, minimal but valid
    let base = Config {
        targets: serde_json::from_value(serde_json::json!([{
            "id": "t", "start_url": "http://127.0.0.1:9/",
            "extract": { "title": { "css": "h1" } }
        }]))
        .unwrap(),
        ..Config::default()
    };
    let mut doc = serde_json::to_value(&base).unwrap();
    doc.as_object_mut().unwrap().remove("config_version");
    std::fs::write(&p, doc.to_string()).unwrap();
    let cfg = Config::load(&p).unwrap();
    assert_eq!(cfg.config_version, crate::migrate::CURRENT_VERSION);
    assert_eq!(cfg.load_warnings.len(), 1, "{:?}", cfg.load_warnings);
    // an unknown key is a hard error, not a shrug
    let mut bad = doc.clone();
    bad.as_object_mut()
        .unwrap()
        .insert("concurrencyy".into(), 4.into());
    std::fs::write(&p, bad.to_string()).unwrap();
    let e = Config::load(&p).unwrap_err().to_string();
    assert!(e.contains("unknown config key `concurrencyy`"), "{e}");
    assert!(e.contains("did you mean `concurrency`"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn production_audit_flags_the_soft_risks_only() {
    let cfg = Config::default();
    // a compliant default config must pass the production audit clean
    assert!(
        cfg.production_audit().is_empty(),
        "{:?}",
        cfg.production_audit()
    );
    let mut loud = cfg.clone();
    loud.respect_robots = false;
    loud.max_retries = 9;
    loud.default_policy.requests_per_sec = 40.0;
    loud.concurrency = 300;
    loud.max_response_bytes = 128 * 1024 * 1024;
    let w = loud.production_audit();
    assert_eq!(w.len(), 5, "{w:?}");
    assert!(w.iter().any(|x| x.contains("respect_robots")));
    assert!(w.iter().any(|x| x.contains("pace")));
    assert!(w.iter().any(|x| x.contains("max_retries")));
    assert!(w.iter().any(|x| x.contains("concurrency")));
    assert!(w.iter().any(|x| x.contains("64 MiB")));
}

fn tempdir_like(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("harvest-cfg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn credential_redaction_covers_parsable_and_opaque_forms() {
    assert_eq!(
        redact_credentials("http://collector:s3cr3t@proxy.corp:8080/exit10"),
        "http://collector:***@proxy.corp:8080"
    );
    assert!(!redact_credentials("http://u:p@h/x?q=1").contains("q=1"));
    assert_eq!(
        redact_credentials("http://plain.host/path"),
        "http://plain.host/path",
        "no credentials => unchanged (nothing to hide)"
    );
    // Unparseable forms must never echo a userinfo or path tail back.
    let r = redact_credentials("socks5://user:pw@10.0.0.1:1080/secret-tail");
    assert!(
        r.contains("***@") && !r.contains("pw") && !r.contains("secret-tail"),
        "{r}"
    );
    assert_eq!(redact_credentials("not a url at all"), "[redacted]");
    assert_eq!(
        redact_url_origin("https://hooks.slack.com/services/T00/B00/XXXXX"),
        "https://hooks.slack.com"
    );
    assert_eq!(redact_url_origin("garbage"), "[redacted]");
}

#[test]
fn env_expansion_only_for_whole_value_refs_and_fails_loud_on_missing() {
    // one test = one process-global env touch, so parallel tests are safe
    let dir = std::env::temp_dir().join(format!("harvest-envvar-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg_path = dir.join("c.json");
    std::env::set_var("HARVEST_TEST_TOKEN", "a-real-token-0123456789");
    std::fs::write(
        &cfg_path,
        format!(
            r#"{{
            "targets": [{{"id":"t","start_url":"http://127.0.0.1:1/x",
              "extract": {{"title": {{"css":"h1"}}}}}}],
            "admin_token": "${{HARVEST_TEST_TOKEN}}",
            "output": {{"kind":"jsonl","path":"{out}"}},
            "metrics_addr": "127.0.0.1:0"
        }}"#,
            out = dir.join("o.jsonl").display()
        ),
    )
    .unwrap();
    let cfg = Config::load(&cfg_path).expect("load with env ref");
    assert_eq!(cfg.admin_token.as_deref(), Some("a-real-token-0123456789"));

    // Partial interpolation must NOT happen: a CSS selector or literal
    // containing ${...} inline stays byte-identical.
    std::fs::write(
        &cfg_path,
        r#"{"targets":[{"id":"t","start_url":"http://x","extract":{"price":{"css":"p[title='${NOT_A_VAR}']"}}}],"output":{"kind":"jsonl","path":"/tmp/o.jsonl"}}"#,
    )
    .unwrap();
    let cfg2 = Config::load(&cfg_path).expect("inline ${...} is literal, not a ref");
    assert_eq!(
        cfg2.targets[0].extract["price"].css,
        "p[title='${NOT_A_VAR}']"
    );

    // A missing variable is a startup error naming the variable — never a
    // silently-literal secret. (Note the exact `${NAME}` form; padded
    // variants are literal text by design, which the case below proves
    // by asserting the error names the variable.)
    std::fs::write(
        &cfg_path,
        r#"{"targets":[{"id":"t","start_url":"http://x","extract":{"t":{"css":"h1"}}}],"admin_token":"${definitely_not_set_VAR}","output":{"kind":"jsonl","path":"/tmp/o.jsonl"}}"#,
    )
    .unwrap();
    let e = format!("{:#}", Config::load(&cfg_path).unwrap_err());
    assert!(e.contains("definitely_not_set_VAR"), "{e}");
    std::env::remove_var("HARVEST_TEST_TOKEN");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn scheduling_defaults_reproduce_legacy_engine_exactly() {
    let cfg = Config::default();
    assert_eq!(cfg.scheduling.mode, "fifo");
    assert_eq!(cfg.scheduling.max_queue_depth, 0, "0 = unbounded queue");
    assert_eq!(cfg.scheduling.per_target_inflight, 0);
    assert_eq!(cfg.scheduling.worker_restarts, 0, "0 = legacy death");
    assert_eq!(cfg.scheduling.job_max_age_secs, 0, "0 = no deadlines");
    assert_eq!(cfg.scheduling.retry_budget_per_min, 0, "0 = unlimited");
    assert!(cfg.transport_breaker.is_none(), "breaker off by default");
    assert!(!cfg.retry_deterministic);
    assert!(cfg.events_path.is_none());
}

#[test]
fn engine_layer_knobs_roundtrip_and_validate() {
    let json = serde_json::json!({
        "targets": [{ "id": "t", "start_url": "http://x/", "extract": { "a": { "css": "h1" } } }],
        "events_path": "out/events.jsonl",
        "retry_deterministic": true,
        "scheduling": {
            "mode": "fair", "max_queue_depth": 500, "per_target_inflight": 2,
            "worker_restarts": 3, "job_max_age_secs": 120, "retry_budget_per_min": 60
        },
        "transport_breaker": { "min_consecutive": 4, "cooldown_secs": 120, "recovery_successes": 2 }
    });
    let mut cfg: Config = serde_json::from_value(json).unwrap();
    cfg.validate().unwrap();
    assert_eq!(cfg.scheduling.mode, "fair");
    assert_eq!(cfg.transport_breaker.as_ref().unwrap().min_consecutive, 4);
    assert!(cfg.retry_deterministic);
}

#[test]
fn engine_layer_footguns_are_rejected() {
    let base = || {
        serde_json::json!({
            "targets": [{ "id": "t", "start_url": "http://x/", "extract": { "a": { "css": "h1" } } }],
        })
    };
    let reject = |patch: serde_json::Value, needle: &str| {
        let mut v = base();
        for (k, val) in patch.as_object().unwrap() {
            v.as_object_mut().unwrap().insert(k.clone(), val.clone());
        }
        let mut cfg: Config = serde_json::from_value(v).unwrap();
        let e = cfg.validate().expect_err("must reject");
        assert!(e.to_string().contains(needle), "{e}");
    };
    reject(
        serde_json::json!({"scheduling": {"mode": "teleport"}}),
        "scheduling.mode",
    );
    reject(
        serde_json::json!({"scheduling": {"per_target_inflight": 65}}),
        "per_target_inflight",
    );
    reject(
        serde_json::json!({"scheduling": {"worker_restarts": 99}}),
        "worker_restarts",
    );
    reject(
        serde_json::json!({"scheduling": {"retry_budget_per_min": 200000}}),
        "retry_budget_per_min",
    );
    reject(
        serde_json::json!({"transport_breaker": {"min_consecutive": 1, "cooldown_secs": 9999, "recovery_successes": 0}}),
        "min_consecutive",
    );
    reject(
        serde_json::json!({"events_path": "out/../etc/passwd"}),
        "events_path",
    );
}

#[test]
fn validate_pins_id_shape_and_uniqueness() {
    let one = || {
        serde_json::from_value::<Vec<TargetCfg>>(serde_json::json!([{
            "id": "shop", "start_url": "http://127.0.0.1:9/",
            "extract": { "title": { "css": "h1" } }
        }]))
        .unwrap()
    };
    let mut cfg = Config {
        targets: one(),
        ..Config::default()
    };
    assert!(
        cfg.validate().is_ok(),
        "baseline: {:?}",
        cfg.validate().err()
    );
    // duplicate ids silently merge two targets' output; refused.
    let mut dup = cfg.clone();
    dup.targets.push(one().remove(0));
    let e = dup.validate().expect_err("dupes must not load");
    assert!(e.to_string().contains("duplicate target id"), "{e}");
    // ids must be id slugs — they become path components and labels.
    let mut ugly = cfg.clone();
    ugly.targets[0].id = "Shop Site!".into();
    let e = ugly.validate().expect_err("bad shape must not load");
    assert!(e.to_string().contains("not usable"), "{e}");
    // proxy ids join the same rules
    let mut twins = cfg.clone();
    twins.proxies = vec![
        ProxyCfg {
            id: "rot".into(),
            url: None,
            region: None,
        },
        ProxyCfg {
            id: "rot".into(),
            url: None,
            region: None,
        },
    ];
    let e = twins.validate().expect_err("twin proxies must not load");
    assert!(e.to_string().contains("duplicate proxy id"), "{e}");
    let _ = &mut cfg;
}

#[test]
fn validate_rejects_insecure_or_contradictory_configs() {
    let base = || -> serde_json::Value {
        serde_json::json!({
            "targets": [{"id":"t","start_url":"http://127.0.0.1:1/x","extract":{"title":{"css":"h1"}}}],
            "output": {"kind":"jsonl","path":"out/x.jsonl"},
            "metrics_addr": "127.0.0.1:9678"
        })
    };
    let mut ok: Config = serde_json::from_value(base()).unwrap();
    ok.validate().expect("baseline must stay valid");

    let mut cases: Vec<(serde_json::Value, &str)> = Vec::new();
    cases.push((
        serde_json::json!({"metrics_addr":"0.0.0.0:9678"}),
        "admin_token",
    ));
    cases.push((serde_json::json!({"metrics_addr":"127.0.0.1"}), "host:PORT"));
    cases.push((
        serde_json::json!({"metrics_addr":"127.0.0.1:99999"}),
        "host:PORT",
    ));
    cases.push((serde_json::json!({"admin_token":"short"}), "at least 12"));
    cases.push((serde_json::json!({"concurrency":513}), "exceeds 512"));
    cases.push((serde_json::json!({"request_timeout_secs":0}), "1..=600"));
    cases.push((serde_json::json!({"connect_timeout_secs":99}), "1..=60"));
    cases.push((serde_json::json!({"max_retries":99}), "hammering"));
    cases.push((serde_json::json!({"max_pages":0}), "collects nothing"));
    cases.push((serde_json::json!({"max_response_bytes":16}), "4096"));
    cases.push((
        serde_json::json!({"proxies":[{"id":"p","url":"ftp://host:21"}]}),
        "scheme",
    ));
    cases.push((serde_json::json!({"redis_url":"http://r:6379"}), "redis://"));
    cases.push((
        serde_json::json!({"alerts":{"webhook_url":"not a url","events":["no_data"],"min_records":1}}),
        "webhook",
    ));
    cases.push((
        serde_json::json!({"audit_path":"../escape/audit.json"}),
        "`..`",
    ));
    cases.push((serde_json::json!({"dlq_path":"out/../../x.dlq"}), "`..`"));
    for (patch, needle) in cases {
        let mut v = base();
        for (k, val) in patch.as_object().unwrap() {
            v[k] = val.clone();
        }
        let mut cfg: Config = serde_json::from_value(v).unwrap();
        let e = cfg.validate().unwrap_err().to_string();
        assert!(
            e.contains(needle),
            "want {needle:?} in error for {patch}: {e}"
        );
    }

    // loopback + strong token is accepted even with 600s timeouts (edge of range)
    let mut v = base();
    v["admin_token"] = serde_json::json!("0123456789abcdef");
    v["request_timeout_secs"] = serde_json::json!(600);
    let mut cfg: Config = serde_json::from_value(v).unwrap();
    cfg.validate().expect("boundaries inclusive");

    // open-http + non-loopback is exactly what used to be possible: no more.
    let mut v = base();
    v["metrics_addr"] = serde_json::json!("0.0.0.0:9678");
    v["admin_token"] = serde_json::json!("0123456789abcdef");
    let mut cfg: Config = serde_json::from_value(v).unwrap();
    cfg.validate()
        .expect("0.0.0.0 is fine WITH a token — the guard is auth, not bind");
}

#[test]
fn postgres_dsn_shape_is_checked_not_leaked() {
    let mut cfg: Config = serde_json::from_value(serde_json::json!({
        "targets": [{"id":"t","start_url":"http://x","extract":{"title":{"css":"h1"}}}],
        "output": {"kind":"postgres","dsn":"host=127.0.0.1 port=5432 user=u password=p"}
    }))
    .unwrap();
    let e = cfg.validate().unwrap_err().to_string();
    // Without the feature, validate() short-circuits on the capability
    // check before the dsn-shape check — either refusal is fine, the
    // invariant under test is that NO path echoes the password.
    if cfg!(feature = "postgres") {
        assert!(e.contains("dbname="), "{e}");
    } else {
        assert!(e.contains("lacks"), "{e}");
    }
    assert!(
        !e.contains("password=p"),
        "validation must not echo the dsn: {e}"
    );
    cfg.output = serde_json::from_value(serde_json::json!({
        "kind":"postgres","dsn":"host=127.0.0.1 dbname=d user=u password=p"
    }))
    .unwrap();
    // (feature-gated binaries may still refuse postgres; only check the
    // error is not about the dsn SHAPE when the feature is missing.)
    if let Err(e) = cfg.validate() {
        assert!(
            e.to_string().contains("lacks") || e.to_string().contains("postgres"),
            "unexpected: {e}"
        );
    }
}
