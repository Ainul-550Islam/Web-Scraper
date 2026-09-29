//! Ops-CLI surface over durable state (no live server): filtering, sorting
//! determinism, jsonl streaming, config sources/redaction, and exit-code
//! semantics. The end-to-end pairing with the admin API lives in
//! `ops_api.rs`; this file pins the fast, offline contract.

use std::io::Write;
use std::process::Command;

use serde_json::{json, Value};

use harvest::testutil::TempFixture;

fn harvest(dir: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_harvest"))
        .args(args)
        // isolate from whatever ambient config/env the dev shell has
        .env_remove("RUST_LOG")
        .env("HOME", dir)
        .current_dir(dir)
        .output()
        .expect("spawn harvest");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn seed_history(path: &std::path::Path) {
    // Six runs: three ok / two error / one cancelled, deliberately out of
    // file order so the sort assertions can't pass by accident.
    let rows = [
        (100u64, "ok", 40u64, 5u64),
        (104, "error", 10, 0),
        (101, "ok", 90, 9),
        (103, "cancelled", 70, 3),
        (102, "error", 20, 1),
        (105, "ok", 55, 7),
    ];
    let mut f = std::fs::File::create(path).unwrap();
    for (ts, status, dur, written) in rows {
        writeln!(
            f,
            "{}",
            json!({
                "run": "clipins",
                "ts": ts,
                "status": status,
                "duration_ms": dur,
                "error": if status == "error" { "boom" } else { "" },
                "summary": {
                    "written": written,
                    "quarantined": 0,
                    "per_target": { "products": { "written": written, "fetched": written } }
                }
            })
        )
        .unwrap();
    }
}

#[test]
fn runs_listing_semantics() {
    let fx = TempFixture::new("cliruns", "http://127.0.0.1:9");
    let hist = fx.dir.join("out/runs.jsonl");
    seed_history(&hist);
    let cfg = fx.cfg_path.to_str().unwrap();

    // default: newest-first, capped by --limit
    let (rc, out, err) = harvest(&fx.dir, &["runs", "--config", cfg, "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["window"], 6);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 6);
    let ts: Vec<u64> = items.iter().map(|i| i["ts"].as_u64().unwrap()).collect();
    assert_eq!(ts, vec![105, 104, 103, 102, 101, 100], "ts desc by default");

    // status filter
    let (rc, out, _) = harvest(
        &fx.dir,
        &["runs", "--config", cfg, "--status", "error", "--json"],
    );
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{v}");
    assert!(items.iter().all(|i| i["status"] == "error"));

    // sort by duration asc is deterministic and matches the seeded order
    let (rc, out, _) = harvest(
        &fx.dir,
        &[
            "runs", "--config", cfg, "--sort", "duration", "--order", "asc", "--json",
        ],
    );
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    let durs: Vec<u64> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["duration_ms"].as_u64().unwrap())
        .collect();
    assert_eq!(durs, vec![10, 20, 40, 55, 70, 90]);
    // ...stable across invocations (same seed order in => same bytes out)
    let (rc, out2, _) = harvest(
        &fx.dir,
        &[
            "runs", "--config", cfg, "--sort", "duration", "--order", "asc", "--json",
        ],
    );
    assert_eq!(rc, 0);
    assert_eq!(out, out2, "byte-identical repeat output");

    // limit + offset page through without touching the window
    let (rc, out, _) = harvest(
        &fx.dir,
        &[
            "runs", "--config", cfg, "--limit", "2", "--offset", "1", "--json",
        ],
    );
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(
        items[0]["ts"], 104,
        "offset applies after sort, window unchanged: {v}"
    );

    // time-range filters are inclusive
    let (rc, out, _) = harvest(
        &fx.dir,
        &[
            "runs", "--config", cfg, "--since", "101", "--until", "103", "--json",
        ],
    );
    assert_eq!(rc, 0);
    let v: Value = serde_json::from_str(&out).unwrap();
    let mut ts: Vec<u64> = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["ts"].as_u64().unwrap())
        .collect();
    ts.sort_unstable();
    assert_eq!(ts, vec![101, 102, 103]);

    // jsonl streams one line per item (valid standalone JSON each)
    let (rc, out, _) = harvest(&fx.dir, &["runs", "--config", cfg, "--jsonl"]);
    assert_eq!(rc, 0);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 6);
    for l in &lines {
        let v: Value = serde_json::from_str(l).expect("each jsonl line is valid JSON");
        assert!(v.get("run").is_some() && v["run"] == "clipins");
    }
    // empty selection streams NOTHING (no empty envelope line)
    let (rc, out, _) = harvest(
        &fx.dir,
        &[
            "runs",
            "--config",
            cfg,
            "--status",
            "cancelled",
            "--until",
            "1",
            "--jsonl",
        ],
    );
    assert_eq!(rc, 0);
    assert_eq!(out.trim(), "");
    // an unknown status is a REJECTED filter (the API's 400 semantics),
    // not a silently empty page — spelling mistakes must not look like data
    let (rc, out, err) = harvest(
        &fx.dir,
        &["runs", "--config", cfg, "--status", "nope", "--json"],
    );
    assert_eq!(rc, 1, "invalid status filter must exit 1, got: {out}");
    assert_eq!(out.trim(), "");
    assert!(err.contains("status"), "error names the knob: {err}");
}

#[test]
fn exit_codes_are_preserved() {
    let fx = TempFixture::new("cliexit", "http://127.0.0.1:9");
    // missing config => load error => exit 1 with a message on stderr
    let (rc, out, err) = harvest(&fx.dir, &["runs", "--config", "nope.json"]);
    assert_eq!(rc, 1, "config load failure must exit 1");
    assert!(out.is_empty(), "errors must not pollute stdout: {out}");
    assert!(err.contains("nope.json"), "{err}");
    // malformed config => validation/parse error => 1
    std::fs::write(fx.dir.join("bad.json"), b"{not json").unwrap();
    let (rc, _, err) = harvest(&fx.dir, &["runs", "--config", "bad.json"]);
    assert_eq!(rc, 1, "{err}");
    // clap-level misuse => 2 (before any file is touched)
    let (rc, out, err) = harvest(&fx.dir, &["runs", "--limit", "not-a-number"]);
    assert_eq!(rc, 2, "parse errors exit 2");
    assert!(out.is_empty());
    assert!(err.contains("invalid value"), "{err}");
    let (rc, _, _) = harvest(&fx.dir, &["no-such-command"]);
    assert_eq!(rc, 2, "unknown subcommand exits 2");
    // bad sort key: validated by the filter layer, surfaces as a clean 1
    let (rc, out, err) = harvest(
        &fx.dir,
        &[
            "runs",
            "--config",
            fx.cfg_path.to_str().unwrap(),
            "--sort",
            "vibes",
        ],
    );
    assert_eq!(rc, 1, "invalid query must exit 1, got {rc} / {err}");
    assert!(out.is_empty(), "the 400-class error goes to stderr only");
    assert!(
        err.contains("sort"),
        "error names the offending knob: {err}"
    );
}

#[test]
fn config_inspect_and_explain() {
    let fx = TempFixture::new("cliconfig", "http://127.0.0.1:9");
    // add a token sourced from the environment to pin both the redaction
    // marker and the `env:` provenance marker
    let mut cfg: Value =
        serde_json::from_str(&std::fs::read_to_string(&fx.cfg_path).unwrap()).unwrap();
    cfg["admin_token"] = json!("${CLIPINS_TOKEN}");
    cfg["max_pages"] = json!(7);
    std::fs::write(&fx.cfg_path, cfg.to_string()).unwrap();

    let cfgp = fx.cfg_path.to_str().unwrap();
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_harvest"))
            .args(args)
            .env("CLIPINS_TOKEN", "sup3r-secret-token-value")
            .output()
            .expect("spawn");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };

    let (rc, out, err) = run(&["config", "inspect", "--config", cfgp, "--json"]);
    assert_eq!(rc, 0, "{err}");
    assert!(
        !out.contains("sup3r-secret-token-value"),
        "inspect leaked the raw secret: {out}"
    );
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["admin_token"], "<redacted>");
    assert_eq!(v["_sources"]["admin_token"], "env:CLIPINS_TOKEN");
    // explicit-vs-default provenance: max_pages set (explicit), others default
    assert_eq!(v["_sources"]["max_pages"], "explicit");
    assert_eq!(v["max_pages"], 7);
    assert_eq!(
        v["_sources"]["user_agent"], "default",
        "unset keys report their provenance"
    );

    // human output must also redact and must not print the token
    let (rc, out, _) = run(&["config", "inspect", "--config", cfgp]);
    assert_eq!(rc, 0);
    assert!(!out.contains("sup3r-secret-token-value"), "{out}");
    assert!(
        out.contains("<redacted>"),
        "table must show the marker: {out}"
    );

    // explain: field filter narrows rows; every row carries doc + default
    let (rc, out, _) = run(&[
        "config",
        "explain",
        "--config",
        cfgp,
        "--field",
        "schedule.",
        "--json",
    ]);
    assert_eq!(rc, 0);
    let rows = serde_json::from_str::<Value>(&out).unwrap()["knobs"].clone();
    let knobs = rows.as_array().unwrap();
    assert!(!knobs.is_empty());
    assert!(knobs
        .iter()
        .all(|k| k["path"].as_str().unwrap().starts_with("schedule.")));
    let every = knobs
        .iter()
        .find(|k| k["path"] == "schedule.every_secs")
        .unwrap();
    assert!(
        every["summary"].as_str().unwrap().len() > 10,
        "real doc text"
    );
    // current value is resolved from the loaded config
    assert_eq!(every["current"], json!(60), "{every}");

    // unknown field => empty listing, still a clean 0 (nothing to say)
    let (rc, out, _) = run(&[
        "config", "explain", "--config", cfgp, "--field", "zzz.", "--json",
    ]);
    assert_eq!(rc, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&out).unwrap()["knobs"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn status_jobs_proxies_offline() {
    let fx = TempFixture::new("clistatus", "http://127.0.0.1:9");
    seed_history(&fx.dir.join("out/runs.jsonl"));
    let cfg = fx.cfg_path.to_str().unwrap();

    let (rc, out, err) = harvest(&fx.dir, &["status", "--config", cfg, "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["run"], "clistatus", "reports the CONFIG name");
    assert_eq!(v["last_run"]["ts"], 105, "newest history line");
    assert_eq!(v["admin"], "disabled", "fixture has no metrics_addr");
    assert!(v["policy_sha256"].as_str().unwrap().len() == 64);

    // human table must render the lane table and the last-run line
    let (rc, out, _) = harvest(&fx.dir, &["status", "--config", cfg]);
    assert_eq!(rc, 0);
    assert!(out.contains("run "), "{out}");
    assert!(out.contains("history"), "lane table: {out}");
    // no raw temp path in ANY human output
    assert!(
        !out.contains("harvest-test-clistatus"),
        "table leaked fixture path: {out}"
    );

    // offline jobs view reads per_target from the latest persisted run
    let (rc, out, err) = harvest(&fx.dir, &["jobs", "--config", cfg, "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    let jobs = v["jobs"]
        .as_array()
        .unwrap_or_else(|| panic!("jobs array in {v}"));
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["target"], "products");
    assert_eq!(
        jobs[0]["written"], 7,
        "written from the newest run's per_target"
    );

    // proxies offline: static config view, clearly labelled
    let (rc, out, err) = harvest(&fx.dir, &["proxies", "--config", cfg, "--json"]);
    assert_eq!(rc, 0, "{err}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert!(v["source"].as_str().unwrap().contains("static view"), "{v}");
    assert_eq!(v["total"], 0);

    // live-only commands must fail CLOSED with exit 1, not hang or fake it
    for cmd in ["workers", "queue", "metrics"] {
        let (rc, out, err) = harvest(&fx.dir, &[cmd, "--config", cfg]);
        assert_eq!(rc, 1, "`harvest {cmd}` without an admin must exit 1");
        assert!(out.is_empty(), "{cmd} wrote junk to stdout: {out}");
        assert!(
            err.contains("admin address") || err.contains("--url"),
            "{cmd} must explain the missing admin: {err}"
        );
    }
    // ...and against a dead address, exit 1 with the unreachable detail
    let (rc, _, err) = harvest(&fx.dir, &["queue", "--config", cfg, "--url", "127.0.0.1:1"]);
    assert_eq!(rc, 1);
    assert!(err.contains("127.0.0.1:1"), "names the target: {err}");
}
