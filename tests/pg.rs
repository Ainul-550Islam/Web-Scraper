#![cfg(feature = "postgres")]
//! E2E against a REAL Postgres cluster (opt-in). The test skips with a
//! printed notice unless HARVEST_TEST_PG=1 and the DSN is reachable, so the
//! suite stays green on machines without a database.
//!
//!   # sandbox example
//!   sudo pg_ctlcluster 17 main start
//!   sudo -u postgres psql -vc "CREATE ROLE harvest LOGIN PASSWORD 'harvest'; CREATE DATABASE harvest OWNER harvest;"
//!   HARVEST_TEST_PG=1 cargo test --features "sqlite postgres" --test pg

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

fn spawn_pages() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut stream) = conn else { continue };
            let Ok(clone) = stream.try_clone() else {
                continue;
            };
            let mut reader = BufReader::new(clone);
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
            let body = match path.as_str() {
                "/robots.txt" => "User-agent: *\nDisallow:\n".to_string(),
                "/a" => "<html><body><h1>Alpha</h1><a href=\"/b\">b</a></body></html>".to_string(),
                "/b" => "<html><body><h1>Beta</h1><a href=\"/a\">a</a></body></html>".to_string(),
                _ => "<html><body>404</body></html>".to_string(),
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    format!("http://127.0.0.1:{port}")
}

fn make_cfg(base: &str, dsn: &str, run: &str) -> harvest::Config {
    let raw = serde_json::json!({
        "name": run,
        "user_agent": "harvest-pgtest/0.1 (contact: qa@example.com)",
        "respect_robots": true,
        "concurrency": 2,
        "max_retries": 0,
        "metrics_addr": "",
        "default_policy": { "requests_per_sec": 5000.0, "burst": 20, "min_interval_ms": 0 },
        "targets": [{
            "id": "pg",
            "start_url": format!("{base}/a"),
            "follow_links": true,
            "max_depth": 1,
            "max_pages": 5,
            "extract": { "title": { "css": "h1" } }
        }],
        "output": { "kind": "postgres", "dsn": dsn }
    });
    let mut cfg: harvest::Config = serde_json::from_value(raw).unwrap();
    cfg.validate().unwrap();
    cfg
}

#[tokio::test]
async fn postgres_sink_roundtrip_and_idempotency() {
    if std::env::var("HARVEST_TEST_PG").as_deref() != Ok("1") {
        eprintln!("SKIP postgres e2e — set HARVEST_TEST_PG=1 and provide a reachable cluster");
        return;
    }
    let dsn = std::env::var("HARVEST_PG_DSN").unwrap_or_else(|_| {
        "host=127.0.0.1 port=5432 user=harvest password=harvest dbname=harvest".into()
    });

    let base = spawn_pages();
    let run = format!("pg-e2e-{}", std::process::id());

    let (admin, conn_task) = {
        let (client, conn) = tokio_postgres::connect(&dsn, tokio_postgres::NoTls)
            .await
            .expect("postgres reachable when HARVEST_TEST_PG=1");
        (
            client,
            tokio::spawn(async move {
                let _ = conn.await;
            }),
        )
    };
    let _ = admin
        .execute(
            &format!("DELETE FROM harvest_records WHERE run = '{run}'"),
            &[],
        )
        .await;

    // run 1: two pages (a + linked b) => two rows
    let summary = harvest::engine::run(make_cfg(&base, &dsn, &run))
        .await
        .unwrap();
    assert_eq!(summary.written, 2, "both pages must persist: {summary:?}");

    // give the sink thread a moment for the final ack path (flush already acked,
    // but rows land microseconds before)
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let row = admin
        .query_one(
            "SELECT count(*), coalesce(string_agg(payload->>'title', ',' ORDER BY id), '') FROM harvest_records WHERE run=$1",
            &[&run],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    let titles: String = row.get(1);
    assert_eq!(count, 2, "exactly 2 rows for run {run}");
    assert!(
        titles.contains("Alpha") && titles.contains("Beta"),
        "titles: {titles}"
    );

    // run 2 with the SAME run name: ON CONFLICT (run,target,fingerprint) => idempotent
    let summary2 = harvest::engine::run(make_cfg(&base, &dsn, &run))
        .await
        .unwrap();
    assert_eq!(summary2.written, 2);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let count2: i64 = admin
        .query_one("SELECT count(*) FROM harvest_records WHERE run=$1", &[&run])
        .await
        .unwrap()
        .get(0);
    assert_eq!(count2, 2, "re-run must not duplicate rows");

    let _ = admin
        .execute(
            &format!("DELETE FROM harvest_records WHERE run = '{run}'"),
            &[],
        )
        .await;
    conn_task.abort();
}
