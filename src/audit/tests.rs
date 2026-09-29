use super::*;

use serde_json::json;

fn cfg() -> Config {
    let raw = json!({
        "name": "audit-unit",
        "user_agent": "harvest-test/0.1 (contact: qa@example.com)",
        "default_policy": { "requests_per_sec": 2.0, "burst": 3, "min_interval_ms": 500 },
        "domain_policies": { "vip.example": { "requests_per_sec": 0.2, "burst": 1, "min_interval_ms": 6000 } },
        "targets": [{ "id": "t", "start_url": "http://x.example/", "extract": { "h": { "css": "h1" } } }],
        "output": { "kind": "null" }
    });
    let mut c: Config = serde_json::from_value(raw).unwrap();
    c.validate().unwrap();
    c
}

#[test]
fn aggregates_match_events_and_policy_echo_scopes_per_host() {
    let rec = Recorder::new();
    let c = cfg();
    rec.note_request("a.example", 200);
    rec.note_request("a.example", 200);
    rec.note_request("a.example", 304);
    rec.note_request("a.example", 429);
    rec.note_request("a.example", 999);
    rec.note_error("a.example");
    rec.note_wait("a.example", Duration::ZERO);
    rec.note_wait("a.example", Duration::from_millis(250));
    rec.note_backoff("a.example", 60_000);
    rec.note_robots_fetched("a.example", true, Some(Duration::from_millis(2500)));
    rec.note_robots_denial("a.example");
    rec.note_written("a.example");
    rec.note_request("vip.example", 404);

    let snap = rec.snapshot(&c);
    let a = &snap["hosts"]["a.example"];
    assert_eq!(a["requests"], json!(5));
    assert_eq!(a["ok200"], json!(2));
    assert_eq!(a["revalidated304"], json!(1));
    assert_eq!(a["blocked"]["429"], json!(1));
    assert_eq!(a["transport_errors"], json!(1));
    assert_eq!(a["wait_permits"], json!(1));
    assert_eq!(a["wait_sleeps"], json!(1));
    assert_eq!(a["wait_slept_ms"], json!(250));
    assert_eq!(a["backoff_ms"], json!(60000));
    assert_eq!(a["robots"]["fetched"], json!(1));
    assert_eq!(a["robots"]["present"], json!(true));
    assert_eq!(a["robots"]["crawl_delay_ms"], json!(2500));
    assert_eq!(a["robots"]["denials"], json!(1));
    assert_eq!(a["records_written"], json!(1));
    // policy echo: default vs per-domain override, scoped per host
    assert_eq!(a["policy"]["requests_per_sec"], json!(2.0));
    assert_eq!(
        snap["hosts"]["vip.example"]["policy"]["min_interval_ms"],
        json!(6000)
    );

    let t = &snap["totals"];
    assert_eq!(t["requests"], json!(6));
    assert_eq!(t["blocked"], json!(1));
    assert_eq!(t["not_found404"], json!(1));
    assert_eq!(t["wait_slept_ms"], json!(250));
    assert_eq!(t["robots_denials"], json!(1));
    assert_eq!(snap["run"], json!("audit-unit"));
    assert_eq!(snap["policy_sha256"], json!(c.hash()));
    assert!(snap["v"] == json!(VERSION));
}

#[test]
fn politeness_and_robots_lifecycle_reach_the_report() {
    let rec = Recorder::new();
    rec.note_wait("h", Duration::ZERO); // permit fast-path, no sleep
    rec.note_wait("h", Duration::from_millis(120));
    rec.note_wait("h", Duration::from_millis(30));
    rec.note_backoff("h", 5000);
    rec.note_error("h");
    rec.note_error("h2");
    rec.note_request("h", 200);
    rec.note_request("h", 304);
    rec.note_request("h", 429);
    rec.note_request("h", 451);
    rec.note_robots_fetched("h", true, Some(Duration::from_secs(2)));
    rec.note_robots_fetched("h2", false, None);
    rec.note_robots_denial("h");
    rec.note_abandoned("h2");
    let s = rec.snapshot(&cfg());
    let t = &s["totals"];
    assert_eq!(t["wait_sleeps"], 2);
    assert_eq!(t["wait_slept_ms"], 150);
    assert_eq!(t["backoff_ms"], 5000);
    assert_eq!(t["transport_errors"], 2);
    assert_eq!(t["blocked"], 2, "429+451 both count as blocked");
    assert_eq!(t["ok200"], 1);
    assert_eq!(t["revalidated304"], 1);
    assert_eq!(t["robots_fetched"], 2);
    assert_eq!(t["robots_denials"], 1);
    let h = &s["hosts"]["h"];
    assert_eq!(h["wait_permits"], 1, "zero-sleep waits counted separately");
    assert_eq!(h["blocked"]["429"], 1, "per-status map kept for operators");
    assert_eq!(h["blocked"]["451"], 1);
    assert_eq!(h["robots"]["present"], true);
    assert_eq!(h["robots"]["crawl_delay_ms"], 2000);
    assert_eq!(s["hosts"]["h2"]["transport_errors"], 1, "hosts isolated");
    assert_eq!(s["hosts"]["h2"]["breaker_abandons"], 1);
    assert_eq!(
        s["hosts"]["h2"]["robots"]["present"], false,
        "404 robots recorded"
    );
}

#[test]
fn atomic_save_and_markdown_render() {
    let dir = std::env::temp_dir().join(format!("harvest-audit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("audit.json");
    let rec = Recorder::new();
    let c = cfg();
    rec.note_request("a.example", 403);
    rec.note_backoff("a.example", 30_000);
    rec.note_robots_fetched("a.example", true, None);
    rec.note_robots_denial("a.example");
    let report = rec.save(&c, &path).unwrap();
    let on_disk: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(on_disk, report, "file equals returned snapshot");
    assert_eq!(
        std::fs::read_dir(&dir).unwrap().count(),
        1,
        "no tmp residue"
    );

    let md = render_markdown(&on_disk);
    assert!(md.contains("# Politeness audit — run audit-unit"), "{md}");
    assert!(md.contains("403×1"), "blocked-by-status visible: {md}");
    assert!(md.contains("30000 ms"), "{md}");
    assert!(md.contains("yes, 1 denied"), "robots denial visible: {md}");
    assert!(
        md.contains("| a.example | 1 | 0 | 0 | 0 | 403×1 | 0 | 0 (0 ms) | 30000 | yes, 1 denied | 0 | 2.0/3 · 500 ms |"),
        "table row exact: {md}"
    );
    // totals sentence present and consistent
    assert!(md.contains("**Totals** — 1 requests"), "{md}");
    std::fs::remove_dir_all(&dir).ok();
}
