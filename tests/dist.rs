//! Live Redis tests — skipped unless HARVEST_TEST_REDIS=1 and a real
//! redis-server is reachable (default redis://127.0.0.1:6399/9, override
//! with HARVEST_TEST_REDIS_URL). These exercise the *actual* network stack:
//! no fakes, no in-memory stand-ins.
#![cfg(feature = "redis")]

use std::sync::Arc;

fn redis_url() -> String {
    std::env::var("HARVEST_TEST_REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6399/9".into())
}

fn skip_unless_enabled() -> bool {
    if std::env::var("HARVEST_TEST_REDIS").ok().as_deref() == Some("1") {
        true
    } else {
        eprintln!("skipping live redis test (set HARVEST_TEST_REDIS=1 + running redis-server)");
        false
    }
}

fn base_cfg(dir: &std::path::Path, start: &str, rps: f64) -> harvest::Config {
    let raw = serde_json::json!({
        "name": "dist-test",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_pages": 50,
        "max_retries": 1,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": rps, "burst": 2, "min_interval_ms": 0 },
        "targets": [{
            "id": "shop",
            "start_url": start,
            "follow_links": true,
            "max_depth": 1,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "jsonl", "path": dir.join("out.jsonl").to_str().unwrap() }
    });
    let mut cfg: harvest::Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    cfg
}

/// Two shards drain one queue against one mock site under a *shared* 3 rps
/// cap, with cross-worker dedup and the robots.txt fetched zero times
/// (pre-seeded Redis cache proves the shared cache path).
#[tokio::test]
async fn two_shards_share_queue_pacing_and_dedup() {
    if !skip_unless_enabled() {
        return;
    }
    use std::io::{BufRead, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let robots_hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = robots_hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = std::io::BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            if path == "/robots.txt" {
                hits2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            let body = if path == "/index.html" {
                let links: Vec<String> = (0..6)
                    .map(|i| format!(r#"<a href="/p{i}">p{i}</a>"#))
                    .collect();
                format!("<html><body><h1>Index</h1>{}</body></html>", links.join(""))
            } else if path.starts_with("/p") {
                format!("<html><body><h1>Item {path}</h1></body></html>")
            } else {
                "<html><body><h1>404</h1></body></html>".to_string()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-dist-{}-1", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prefix = format!("h:t1:{}", std::process::id());
    let cfg = base_cfg(&dir, &format!("http://127.0.0.1:{port}/index.html"), 3.0);

    let mut q = harvest::dist::Queue::connect(&redis_url(), &prefix)
        .await
        .unwrap();
    q.flush_prefix().await.unwrap();
    // Pre-seed the robots answer: a correct shared cache means NO shard
    // should ever hit /robots.txt.
    q.robots_store(
        &format!("robots:http://127.0.0.1:{port}"),
        Some("User-agent: *\nDisallow:\n"),
        600,
    )
    .await
    .unwrap();

    let seeded = harvest::dist::push(&cfg, &redis_url(), &prefix)
        .await
        .unwrap();
    assert_eq!(seeded, 1);

    let mk_opts = || harvest::dist::WorkerOpts {
        name: "w-test".into(),
        queue_fail_limit: 10,
        redis_url: redis_url(),
        prefix: prefix.clone(),
        idle_secs: 4,
        stop: None,
    };
    let t0 = std::time::Instant::now();
    let (wa, wb) = tokio::join!(
        harvest::dist::worker(cfg.clone(), mk_opts()),
        harvest::dist::worker(cfg.clone(), mk_opts())
    );
    let elapsed = t0.elapsed();
    let sa = wa.unwrap();
    let sb = wb.unwrap();

    // Every page collected exactly once across the two shards.
    let written = sa.written + sb.written;
    assert_eq!(written, 7, "index + 6 products, shard stats {sa:?} {sb:?}");
    assert_eq!(
        sa.deduped + sb.deduped,
        0,
        "unique bodies must not dedup: {sa:?} {sb:?}"
    );

    // Shared limiter proof: 7 page requests at 3 rps cannot finish inside
    // two 1-second windows — a per-process limiter would blow through this.
    assert!(
        elapsed.as_millis() >= 900,
        "shared pacing must serialize requests, elapsed {elapsed:?}"
    );

    // Shared robots cache proof: zero hits on /robots.txt.
    assert_eq!(
        robots_hits.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "robots must come from redis cache"
    );

    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(counts["jobs"], 0, "queue drained: {counts}");
    assert_eq!(counts["inflight"], 0, "everything acked: {counts}");
    assert_eq!(counts["dlq"], 0, "no dead jobs: {counts}");

    let mut lines = 0;
    for f in std::fs::read_dir(&dir).unwrap().flatten() {
        if f.file_name().to_string_lossy().starts_with("out.") {
            lines += std::fs::read_to_string(f.path()).unwrap().lines().count();
        }
    }
    assert_eq!(
        lines, 7,
        "shard fragments must merge to the full result set"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A permanently-500 URL is requeued once (max_retries=1) then dead-lettered
/// — queue-level retry, not fetcher-level hammering.
#[tokio::test]
async fn queue_retries_then_dead_letters() {
    if !skip_unless_enabled() {
        return;
    }
    use std::io::{BufRead, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hits2 = hits.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = std::io::BufReader::new(clone);
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr).is_err() || hdr == "\n" || hdr == "\r\n" {
                    break;
                }
            }
            let (status, body) = if path == "/robots.txt" {
                ("200 OK", "User-agent: *\nDisallow:\n".to_string())
            } else {
                hits2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                ("503 Service Unavailable", "gone".to_string())
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let dir = std::env::temp_dir().join(format!("harvest-dist-{}-2", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let prefix = format!("h:t2:{}", std::process::id());
    let cfg = base_cfg(&dir, &format!("http://127.0.0.1:{port}/broken"), 100.0);
    let mut q = harvest::dist::Queue::connect(&redis_url(), &prefix)
        .await
        .unwrap();
    q.flush_prefix().await.unwrap();
    harvest::dist::push(&cfg, &redis_url(), &prefix)
        .await
        .unwrap();

    let st = harvest::dist::worker(
        cfg.clone(),
        harvest::dist::WorkerOpts {
            name: "w-test2".into(),
            queue_fail_limit: 10,
            redis_url: redis_url(),
            prefix: prefix.clone(),
            idle_secs: 3,
            stop: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(st.requeued, 1, "one queue-level retry allowed: {st:?}");
    assert_eq!(st.dead, 1, "then dead-lettered: {st:?}");
    // attempts 0 + 1 => exactly two page requests, no more (no hammering).
    assert_eq!(
        hits.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "retry budget must be honored"
    );

    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(
        counts["dlq"], 1,
        "one entry parked on the redis dlq: {counts}"
    );
    let dlq: Vec<String> = redis::cmd("LRANGE")
        .arg(&[format!("{prefix}:dlq"), "0".to_string(), "-1".to_string()])
        .query_async(&mut redis_con(&redis_url()).await)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&dlq[0]).unwrap();
    assert!(v["reason"].as_str().unwrap().starts_with("failed:"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn reliable_queue_semantics_on_live_redis() {
    if !skip_unless_enabled() {
        return;
    }
    let prefix = format!("h:t3:{}", std::process::id());
    let mut q = harvest::dist::Queue::connect(&redis_url(), &prefix)
        .await
        .unwrap();
    q.flush_prefix().await.unwrap();

    let job = harvest::dist::Job {
        expires_at: None,
        url: "http://127.0.0.1:9/one".into(),
        depth: 0,
        target: 0,
        attempts: 0,
    };
    q.push_job(&job).await.unwrap();
    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(counts["jobs"], 1);

    // A popped-but-unacked job is *visible in inflight* (crash-safe)...
    let (raw, popped) = q
        .pop_job(std::time::Duration::from_millis(200))
        .await
        .unwrap()
        .expect("job");
    assert_eq!(popped.url, job.url);
    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(counts["jobs"], 0, "left jobs");
    assert_eq!(counts["inflight"], 1, "parked in inflight until ack");

    // ...and a startup sweep (no live workers) returns it to the queue.
    let sweep = q.sweep_inflight().await.unwrap();
    assert_eq!(sweep.moved, 1);
    assert_eq!(sweep.held_by_lease, 0);
    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(counts["jobs"], 1);
    assert_eq!(counts["inflight"], 0);

    // Re-pop + ack empties both lists.
    let (raw2, _) = q
        .pop_job(std::time::Duration::from_millis(200))
        .await
        .unwrap()
        .expect("job again");
    q.ack(&raw2).await.unwrap();
    let counts: serde_json::Value = q.counts().await.unwrap();
    assert_eq!(counts["jobs"], 0);
    assert_eq!(counts["inflight"], 0);
    let _ = raw;

    // Requeue bumps attempts; dedup set answers "first time?" exactly once.
    q.requeue(job.clone()).await.unwrap();
    let (_, back) = q
        .pop_job(std::time::Duration::from_millis(200))
        .await
        .unwrap()
        .expect("requeued");
    assert_eq!(back.attempts, 1);

    assert!(q.mark_seen("seen_urls", "dedup-me").await.unwrap());
    assert!(!q.mark_seen("seen_urls", "dedup-me").await.unwrap());
    q.flush_prefix().await.unwrap();
}

async fn redis_con(url: &str) -> redis::aio::MultiplexedConnection {
    redis::Client::open(url.to_string())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap()
}

#[tokio::test]
async fn bad_queue_prefix_is_refused_before_any_io() {
    // Key concatenation safety: nothing reaches the network either.
    assert!(
        harvest::dist::Queue::connect("redis://127.0.0.1:1", "ok-prefix")
            .await
            .is_err(),
        "port 1: connection should fail, not the prefix check"
    );
    assert!(
        harvest::dist::Queue::connect("redis://127.0.0.1:1", "a:b:c:tenant-7")
            .await
            .is_err(),
        "colons are legal — the failure must be the unreachable server"
    );
    for bad in ["injection*", "a b", "", "{db0}", "x;y", "p/refix"] {
        let Err(e) = harvest::dist::Queue::connect("redis://127.0.0.1:1", bad).await else {
            panic!("prefix {bad:?} must be rejected");
        };
        assert!(e.to_string().contains("prefix"), "{bad:?}: {e}");
    }
}

#[tokio::test]
async fn leases_shelter_live_jobs_from_the_sweep() {
    if !skip_unless_enabled() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("harvest-lease-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let qurl = redis_url();
    let mut q = harvest::dist::Queue::connect(&qurl, "hrl")
        .await
        .unwrap()
        .with_lease_ttl(1);
    q.flush_prefix().await.unwrap();
    let job = harvest::dist::Job {
        url: "http://127.0.0.1:9/p".into(),
        depth: 0,
        target: 0,
        attempts: 0,
        expires_at: None,
    };
    q.push_job(&job).await.unwrap();
    let (_raw, _) = q
        .pop_job(std::time::Duration::from_millis(300))
        .await
        .unwrap()
        .unwrap();
    // unleased: sweep reclaims
    assert_eq!(q.sweep_inflight().await.unwrap().moved, 1);
    // re-pop, hold a lease: sweep must skip it, and the holder is named
    let (raw, _) = q
        .pop_job(std::time::Duration::from_millis(300))
        .await
        .unwrap()
        .unwrap();
    let wid = harvest::ids::WorkerId::parse("w-lease").unwrap();
    let lease = harvest::ids::LeaseId::new(&wid, 1, 0xabcd);
    q.touch_lease(&raw, &lease).await.unwrap();
    assert_eq!(
        q.lease_holder(&raw).await.unwrap().as_deref(),
        Some(lease.as_str())
    );
    let s = q.sweep_inflight().await.unwrap();
    assert_eq!(
        (s.moved, s.held_by_lease),
        (0, 1),
        "live lease shelters the job"
    );
    // after the TTL passes with no heartbeat, it is reclaimable again
    tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
    let s = q.sweep_inflight().await.unwrap();
    assert_eq!(
        (s.moved, s.held_by_lease),
        (1, 0),
        "expired lease releases the stray"
    );
    q.flush_prefix().await.unwrap();
    std::fs::remove_dir_all(&dir).ok();
}
