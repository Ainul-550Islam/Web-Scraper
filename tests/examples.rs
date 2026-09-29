//! Every config committed under `examples/` (plus the root
//! `config.example.json`) must load through the REAL `Config` parser — the
//! same one `harvest validate` runs. If a doc example ever drifts from the
//! schema, CI fails here before a human copies it into production.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn offline_examples_load_through_the_real_parser() {
    for name in [
        "config.example.json",
        "examples/config.basic.json",
        "examples/config.pipeline.json",
        "examples/config.distributed.json",
    ] {
        let path = repo_root().join(name);
        match harvest::config::Config::load(&path) {
            Ok(cfg) => assert!(
                !cfg.targets.is_empty() || name.contains("distributed"),
                "{name} parsed to zero targets"
            ),
            Err(e) => panic!("{name} failed to load: {e}"),
        }
    }
}

#[test]
fn production_example_needs_env_secrets_and_succeeds_once_present() {
    let path = repo_root().join("examples/config.production.json");

    // Without the env vars the loader must fail LOUDLY — `${VAR}` references
    // never silently expand to empty strings.
    let err = harvest::config::Config::load(&path)
        .expect_err("unexpanded ${VAR} references must abort the load");
    let msg = err.to_string();
    assert!(
        msg.contains("undefined environment variable"),
        "unexpected failure mode: {msg}"
    );

    // With realistic dummy values it must fully validate, token policy and
    // all (a 9-char token below the entropy floor must also fail — pinned in
    // unit tests; here we just use a long dummy).
    std::env::set_var("ALERT_WEBHOOK", "https://example.invalid/hook");
    std::env::set_var(
        "HARVEST_ADMIN_TOKEN",
        "dummy-value-long-enough-for-the-entropy-gate",
    );
    let cfg = harvest::config::Config::load(&path).expect("production example with env set");
    assert!(!cfg.targets.is_empty());
}
