use super::*;

#[test]
fn expire_at_follows_job_max_age_cap() {
    let mut cfg = Config::default();
    assert_eq!(expire_at(&cfg), None, "default = legacy never-expire");
    cfg.scheduling.job_max_age_secs = 60;
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let exp = expire_at(&cfg).unwrap();
    assert!(
        exp >= before + 60 && exp <= before + 62,
        "{exp} vs {before}"
    );
}

#[test]
fn job_payload_is_backward_and_forward_compatible() {
    // An older seed (or a hand-pushed job) has no attempts/expires_at:
    // they must default, not fail the pop.
    let old: Job = serde_json::from_str(r#"{"url":"http://x/","depth":1,"target":0}"#).unwrap();
    assert_eq!(old.attempts, 0);
    assert_eq!(old.expires_at, None);
    let json = serde_json::to_string(&old).unwrap();
    let back: Job = serde_json::from_str(&json).unwrap();
    assert_eq!(back.url, "http://x/");
    assert_eq!(back.depth, 1);
    // explicit expiry survives the round trip (workers dead-letter on it)
    let j = Job {
        url: "http://x/".into(),
        depth: 0,
        target: 3,
        attempts: 2,
        expires_at: Some(99),
    };
    let v: serde_json::Value = serde_json::from_str(&serde_json::to_string(&j).unwrap()).unwrap();
    assert_eq!(v["attempts"], 2);
    assert_eq!(v["expires_at"], 99);
    assert!(v["url"].is_string(), "payload stays human-inspectable JSON");
}

#[test]
fn dedup_key_normalizes_scheme_and_fragment() {
    let a = dedup_key(&Url::parse("http://e.com/p/i.html?x=1#top").unwrap());
    let b = dedup_key(&Url::parse("https://e.com/p/i.html?x=1").unwrap());
    assert_eq!(a, b, "scheme/fragment must not fork the dedup lane");
    let c = dedup_key(&Url::parse("https://e.com/p/i.html?x=2").unwrap());
    assert_ne!(a, c, "query is part of identity");
    let d = dedup_key(&Url::parse("https://e.com/p/i.html").unwrap());
    assert_ne!(a, d, "bare path differs from any query");
}

#[test]
fn shard_output_only_forks_file_sinks() {
    let pid = std::process::id();
    let mut cfg = Config {
        output: crate::config::SinkCfg::Jsonl {
            path: "out/run.jsonl".into(),
        },
        ..Default::default()
    };
    shard_output(&mut cfg);
    match &cfg.output {
        crate::config::SinkCfg::Jsonl { path } => {
            assert_eq!(
                path.file_name().unwrap().to_string_lossy(),
                format!("run.p{pid}.jsonl")
            );
        }
        other => panic!("sharded to {other:?}"),
    }
    // csv keeps its extension, sqlite is process-safe and stays shared
    cfg = Config {
        output: crate::config::SinkCfg::Csv {
            path: "out/run.csv".into(),
        },
        ..Default::default()
    };
    shard_output(&mut cfg);
    match &cfg.output {
        crate::config::SinkCfg::Csv { path } => {
            assert!(path.extension().unwrap() == "csv");
        }
        other => panic!("csv sharded to {other:?}"),
    }
    cfg = Config {
        output: crate::config::SinkCfg::Sqlite {
            path: "db.sqlite".into(),
        },
        ..Default::default()
    };
    shard_output(&mut cfg);
    assert!(
        matches!(cfg.output, crate::config::SinkCfg::Sqlite { ref path } if path.as_os_str() == "db.sqlite"),
        "sqlite/postgres sinks must NOT be forked per process"
    );
}
