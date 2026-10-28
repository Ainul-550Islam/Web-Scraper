use super::*;

use serde_json::json;
use url::Url;

fn cfg_json(extra: serde_json::Value) -> crate::Config {
    let raw = json!({
        "name": "ck",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "default_policy": { "requests_per_sec": 1.0 },
        "targets": [
            { "id": "a", "start_url": "http://127.0.0.1:1/index.html",
              "extract": { "t": { "css": "h1" } } },
            { "id": "b", "start_url": "http://127.0.0.1:1/feed.json", "mode": "json",
              "extract": { "n": { "path": "$.n" } } }
        ],
        "output": { "kind": "null" }
    });
    let mut raw = raw;
    if let (Some(o), Some(e)) = (raw.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            o.insert(k.clone(), v.clone());
        }
    }
    let mut cfg: crate::Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    cfg
}

fn sample() -> Checkpoint {
    Checkpoint {
        v: VERSION,
        run: "ck".into(),
        policy_sha256: String::new(),
        saved_ts: 1,
        global_pages: 7,
        pending: vec![PendingJob {
            url: "http://127.0.0.1:1/p/9".into(),
            depth: 2,
            target: 0,
        }],
        targets: vec![
            TargetState {
                id: "a".into(),
                visited: vec!["127.0.0.1/index.html".into(), "127.0.0.1/p/9".into()],
                fp: vec!["deadbeef".into()],
                written: 3,
            },
            TargetState {
                id: "b".into(),
                visited: vec![],
                fp: vec![],
                written: 4,
            },
        ],
    }
}

#[test]
fn round_trip_atomic_save() {
    let dir = std::env::temp_dir().join(format!("harvest-ckpt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.ckpt");
    let cfg = cfg_json(json!({}));
    let mut ck = sample();
    ck.policy_sha256 = cfg.hash();
    ck.save(&path).unwrap();
    // temp file must be gone (renamed), content valid, load matches
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "no stray tmp files"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let got = Checkpoint::load_validated(&path, &cfg).unwrap();
    assert_eq!(got.global_pages, 7);
    assert_eq!(got.pending[0].url, "http://127.0.0.1:1/p/9");
    assert_eq!(got.targets[0].visited.len(), 2);
    assert_eq!(got.targets[1].written, 4);
    std::fs::remove_dir_all(&dir).ok();
}

#[cfg(unix)]
#[test]
fn checkpoint_loader_refuses_symlinked_state_files() {
    use std::os::unix::fs::symlink;

    let dir = std::env::temp_dir().join(format!("harvest-ckpt-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim.json");
    let link = dir.join("state.ckpt");
    std::fs::write(&victim, b"{}\n").unwrap();
    symlink(&victim, &link).unwrap();

    let err = Checkpoint::load_validated(&link, &cfg_json(json!({}))).unwrap_err();
    assert!(err.to_string().contains("read checkpoint"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn policy_drift_and_shape_mismatch_are_refused() {
    let dir = std::env::temp_dir().join(format!("harvest-ckpt-drift-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.ckpt");

    // Written under config X...
    let cfg_x = cfg_json(json!({}));
    let mut ck = sample();
    ck.policy_sha256 = cfg_x.hash();
    ck.save(&path).unwrap();

    // ...resumed under Y (concurrency changed = politeness-adjacent drift).
    let cfg_y = cfg_json(json!({ "concurrency": 9 }));
    let err = Checkpoint::load_validated(&path, &cfg_y)
        .unwrap_err()
        .to_string();
    assert!(err.contains("config changed since"), "{err}");

    // Run-name mismatch is caught first and clearly.
    let cfg_z = cfg_json(json!({ "name": "other" }));
    let err = Checkpoint::load_validated(&path, &cfg_z)
        .unwrap_err()
        .to_string();
    assert!(err.contains("belongs to run"), "{err}");

    // Garbage bytes => "corrupt", not a panic and not a silent fresh start.
    std::fs::write(&path, b"{not json").unwrap();
    let err = Checkpoint::load_validated(&path, &cfg_x)
        .unwrap_err()
        .to_string();
    assert!(err.contains("corrupt checkpoint"), "{err}");

    // Unsupported version is refused with a pointer.
    let mut old = ck;
    old.v = VERSION + 1;
    old.save(&path).unwrap();
    let err = Checkpoint::load_validated(&path, &cfg_x)
        .unwrap_err()
        .to_string();
    assert!(err.contains("version"), "{err}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn pending_jobs_survive_serialize_with_parseable_urls() {
    let ck = sample();
    let s = serde_json::to_string(&ck).unwrap();
    let back: Checkpoint = serde_json::from_str(&s).unwrap();
    let u = Url::parse(&back.pending[0].url).unwrap();
    assert_eq!(u.path(), "/p/9");
    assert!(back.has_pending());
    assert!(!Checkpoint {
        pending: vec![],
        ..back
    }
    .has_pending());
}
