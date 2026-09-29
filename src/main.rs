use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use harvest::Config;
use serde_json::json;
use std::fmt::Write as _;
use url::Url;

/// harvest — a compliance-first Rust data-collection engine.
///
/// Polite on purpose: robots.txt enforced, per-domain pacing, refusals are
/// honored (backoff), and the proxy pool is for reliability/geography only.
#[derive(Parser)]
#[command(name = "harvest", version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a collection job defined by a JSON config file.
    Run {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Override the global page budget for this run.
        #[arg(long)]
        max_pages: Option<usize>,
        /// Additionally seed URLs from the dead-letter file; entries that
        /// succeed are cleared, entries that fail get attempts+1.
        #[arg(long)]
        retry_dlq: bool,
        /// Continue an interrupted crawl from the config's `ckpt_path`.
        /// Refused if the config (policy sha) changed since the checkpoint;
        /// a complete run deletes the file, so re-running without --resume
        /// after finishing simply starts fresh.
        #[arg(long)]
        resume: bool,
    },
    /// Scheduler: repeat the job every schedule.every_secs until stopped
    /// (schedule.runs caps the count; 0 = forever). Ctrl-c stops after the
    /// current run; the dashboard and history stay available between runs.
    Serve {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        every_secs: Option<u64>,
        #[arg(long)]
        runs: Option<usize>,
    },
    /// Show the most recent entries from the run-history log.
    History {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long, default_value = "10")]
        n: usize,
    },
    /// Validate a config file (selectors, proxies, URLs) without fetching.
    Validate {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
    },
    /// Print the derived JSON-Schema (draft-07) for a target's records:
    /// declared types where the config has a `schema`, types inferred from
    /// transform chains elsewhere, plus `harvest:*` annotations of the
    /// extraction rules. Pipe to `check-jsonschema` or a diff tool to gate
    /// record-format changes in CI.
    Schema {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Only this target id (default: every target, one doc per line).
        #[arg(long)]
        target: Option<String>,
    },
    /// One-shot fetch: get a single URL and print status + size. Good for
    /// checking an egress/proxy path before committing to a run.
    Fetch { url: String },
    /// Distributed mode (needs `--features redis`): seed the shared Redis
    /// crawl queue with each target's start URL. Then run N workers anywhere.
    #[cfg(feature = "redis")]
    Push {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        redis: Option<String>,
        #[arg(long, default_value = "harvest")]
        prefix: String,
        /// Clear {prefix}:* keys first (fresh run).
        #[arg(long)]
        reset: bool,
    },
    /// Distributed mode: drain the shared queue as one shard until the queue
    /// stays empty for --idle-secs seconds (or ctrl-c). Each shard writes its
    /// own sink fragment (jsonl/csv get a .p<pid> suffix automatically).
    #[cfg(feature = "redis")]
    Worker {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        redis: Option<String>,
        #[arg(long, default_value = "harvest")]
        prefix: String,
        #[arg(long, default_value_t = 5)]
        idle_secs: u64,
        /// This shard's name (slug); keys lease records + logs.
        /// Default: w-<pid>.
        #[arg(long)]
        name: Option<String>,
    },
    /// Queue state. Default: LIVE scheduler queue via the admin API (needs
    /// a running serve or worker). On a redis build, `--redis redis://…`
    /// (or `redis_url` in the config) prints distributed queue counters —
    /// jobs/inflight/dlq/dedup sets — instead.
    #[cfg(not(feature = "redis"))]
    Queue {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Admin address host:port of a running serve (default: config).
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Queue state. Default: LIVE scheduler queue via the admin API (needs
    /// a running serve or worker). With `--redis redis://…` (or `redis_url`
    /// in the config) this prints the distributed Redis queue counters —
    /// jobs/inflight/dlq/dedup sets — instead.
    #[cfg(feature = "redis")]
    Queue {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Admin address host:port of a running serve (default: config).
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        redis: Option<String>,
        #[arg(long, default_value = "harvest")]
        prefix: String,
    },
    /// Ops snapshot: config, storage lanes, and the last recorded run.
    /// `--live` also merges the running admin API (`/diagnostics`, `/stats`)
    /// served on `metrics_addr` (or `--url`).
    Status {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Admin address host:port of a running serve (default: config).
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        live: bool,
        #[arg(long)]
        json: bool,
    },
    /// Health probe. Offline: validates the config and checks storage
    /// lanes. `--url` (or config metrics_addr) probes a live admin
    /// `/health`; exit code is 0 on healthy, 1 otherwise, so cron/systemd
    /// can gate on it.
    Health {
        /// Config to read defaults from (optional: `--url` alone works).
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Per-target job view: last recorded run tallies offline, live
    /// engine status with `--url`.
    Jobs {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
    },
    /// List recent runs from the history log with filtering, sorting, and
    /// bounded paging (same engine as the admin /runs endpoint).
    Runs {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        /// ok | error | cancelled
        #[arg(long)]
        status: Option<String>,
        /// ts | duration | written
        #[arg(long)]
        sort: Option<String>,
        /// asc | desc (default desc)
        #[arg(long)]
        order: Option<String>,
        /// Unix seconds bounds (inclusive)
        #[arg(long)]
        since: Option<u64>,
        #[arg(long)]
        until: Option<u64>,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
    },
    /// Worker pool state (live supervisor snapshot; needs a running serve).
    Workers {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Proxy pool: configured egresses offline, EWMA health with `--url`.
    Proxies {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Engine metrics: JSON stats (default), Prometheus text (`--prom`),
    /// straight from a running admin API.
    Metrics {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        prom: bool,
    },
    /// Inspect the dataset sink directory: partitions, parts, bytes
    /// (offline filesystem view; `dt=` filter supported).
    Datasets {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Only this partition key (e.g. dt=2026-01-31).
        #[arg(long)]
        dt: Option<String>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
    },
    /// Config intelligence: source (default/explicit/env) + redaction-safe
    /// effective values, or explain any knob.
    Config {
        #[command(subcommand)]
        cmd: ConfigCmd,
    },
    /// Render the politeness audit report (per-host receipts written by a
    /// run with `audit_path` set). `--md` gives a human summary; default is
    /// the machine-readable JSON.
    Audit {
        /// audit.json produced by `harvest run`
        #[arg(long)]
        file: PathBuf,
        /// Render markdown instead of pretty JSON
        #[arg(long)]
        md: bool,
    },
    /// Client delivery: convert engine JSONL output into a folder with
    /// records.csv + records.jsonl + manifest.json (per-file sha256, record
    /// counts, and — with --history — the run's policy hash). Deterministic:
    /// same input => byte-identical files. Push the folder with rclone; the
    /// manifest travels with it.
    Export {
        /// Engine JSONL output file(s); repeatable.
        #[arg(long)]
        from: Vec<PathBuf>,
        /// Destination directory (created; existing files are replaced).
        #[arg(long)]
        to: PathBuf,
        /// One csv+jsonl pair per target instead of combined records files.
        #[arg(long)]
        split_target: bool,
        /// Run-history log (schedule.history_path) to embed config_sha256.
        #[arg(long)]
        history: Option<PathBuf>,
        /// HMAC key file: writes manifest.json.sig next to the manifest.
        #[arg(long)]
        key: Option<PathBuf>,
        /// Byte cap per part file for large deliveries; cuts happen at
        /// record boundaries (0 = one file per stream).
        #[arg(long, default_value_t = 0)]
        max_part_bytes: u64,
        /// Resume an interrupted export: parts whose sha256 still validate
        /// are kept; everything else is regenerated.
        #[arg(long)]
        resume: bool,
        /// Delivery format(s), repeatable or comma-separated: csv, jsonl,
        /// xlsx, pdf, parquet, xml, zip (default: csv,jsonl).
        #[arg(long = "format", value_name = "FORMAT")]
        format: Vec<String>,
        /// Dataset id used for file names: `dataset-<id>.<ext>` (sanitized).
        /// Defaults to the historical `records` / `<target>` naming.
        #[arg(long)]
        dataset: Option<String>,
    },
    /// Verify a delivered export folder against its manifest.json: presence,
    /// bytes, sha256, line counts, record totals — and the detached manifest
    /// signature when --key is given. Exit code 1 on any mismatch.
    Verify {
        /// Export directory (the one `--to` pointed at).
        #[arg(long)]
        dir: PathBuf,
        /// Public/attest key file used at signing time.
        #[arg(long)]
        key: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Attestation: generate HMAC keys, verify detached .sig evidence files
    Attest {
        /// Create a fresh 32-byte hex key here (mode 0600; refuses overwrite)
        #[arg(long)]
        gen_key: Option<PathBuf>,
        /// Verify this file (e.g. out/audit.json or deliver/manifest.json)
        #[arg(long)]
        verify: Option<PathBuf>,
        /// Signature file (default: <verify>.sig)
        #[arg(long)]
        sig: Option<PathBuf>,
        /// Key file used to sign
        #[arg(long)]
        key: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show the effective config redacted (secrets are `<redacted>` markers,
    /// never values), with per-key source: default | explicit | env:VAR.
    Inspect {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Explain knobs: type, default, and what they do — plus the current
    /// (redacted) value when a config file is given. `--field` filters.
    Explain {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        /// Substring filter on the dotted path (e.g. `--field alerts`).
        #[arg(long)]
        field: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Migrate a config file to the current schema version (see
    /// `harvest::migrate`). Deterministic and conservative: unknown keys
    /// are refused (typo detection), newer-than-this-build files refuse to
    /// load, and `${ENV}` references stay untouched — secrets are never
    /// rewritten to disk. `--out` writes the stamped document; without it
    /// the migration is reported only (`--dry-run` forces that).
    Migrate {
        #[arg(long, default_value = "config.json")]
        config: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        json: bool,
    },
}

/// Resolve the admin address and optional bearer token for `--url`-style
/// commands: explicit url wins, else the config's metrics_addr; the token is
/// the config value (env-expanded at load) and is never printed.
fn admin_target(cfg: &harvest::Config, url: &Option<String>) -> Result<(String, Option<String>)> {
    let addr = url
        .clone()
        .or_else(|| {
            if cfg.metrics_addr.is_empty() {
                None
            } else {
                Some(cfg.metrics_addr.clone())
            }
        })
        .context("no admin address: pass --url HOST:PORT or set metrics_addr")?;
    Ok((addr, cfg.admin_token.clone()))
}

/// Print a Value as pretty JSON (--json), compact one-line, or a rendered
/// table via `render` for human mode.
fn print_mode(v: &serde_json::Value, json: bool, jsonl: bool, render: impl FnOnce() -> String) {
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
        );
    } else if jsonl {
        // Streaming mode is for `jq`/pipes: a bare array — or a listing
        // object with an `items` array — emits one line per item; anything
        // else is a single line.
        let arr = match v {
            serde_json::Value::Array(items) => Some(items),
            serde_json::Value::Object(m) => match m.get("items") {
                Some(serde_json::Value::Array(items)) => Some(items),
                _ => None,
            },
            _ => None,
        };
        match arr {
            Some(items) => {
                for it in items {
                    println!("{it}");
                }
            }
            None => println!("{v}"),
        }
    } else {
        print!("{}", render());
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Classic unix pipe semantics: Rust ignores SIGPIPE, so `harvest history
    // --n 20 | head` would panic on the broken println! instead of exiting
    // the way every shell tool does. Reset to default disposition first.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,harvest=debug".into());
    // Data plane discipline: stdout is ONLY command output (JSON, tables,
    // reports). Logs — including the `aborted` line on failure — go to
    // stderr so `harvest … > file` and `| jq` never see log noise.
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(err = format!("{e:#}"), "aborted");
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.cmd {
        #[cfg(feature = "redis")]
        Cmd::Push {
            config,
            redis,
            prefix,
            reset,
        } => {
            let cfg = Config::load(&config)?;
            let url = redis
                .or_else(|| cfg.redis_url.clone())
                .context("pass --redis redis://… or set redis_url in the config")?;
            let mut q = harvest::dist::Queue::connect(&url, &prefix)
                .await
                .with_context(|| format!("redis {}", harvest::config::redact_credentials(&url)))?;
            if reset {
                q.flush_prefix().await?;
            }
            let n = harvest::dist::push(&cfg, &url, &prefix).await?;
            println!(
                "seeded {n} start job(s) under `{prefix}` — {}",
                q.counts().await?
            );
            Ok(())
        }
        #[cfg(feature = "redis")]
        Cmd::Worker {
            config,
            redis,
            prefix,
            idle_secs,
            name,
        } => {
            let cfg = Config::load(&config)?;
            let url = redis
                .or_else(|| cfg.redis_url.clone())
                .context("pass --redis redis://… or set redis_url in the config")?;
            let stop = Arc::new(AtomicBool::new(false));
            let stop_sig = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::warn!("stop requested — worker will finish the current job and exit");
                stop_sig.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let st = harvest::dist::worker(
                cfg,
                harvest::dist::WorkerOpts {
                    redis_url: url,
                    prefix,
                    name: name.unwrap_or_else(|| format!("w-{}", std::process::id())),
                    idle_secs,
                    queue_fail_limit: 60,
                    stop: Some(stop),
                },
            )
            .await?;
            println!("{}", serde_json::to_string(&st)?);
            Ok(())
        }
        Cmd::Status {
            config,
            url,
            live,
            json,
        } => {
            let cfg = Config::load(&config)?;
            let mut v = serde_json::json!({
                "run": cfg.name,
                "policy_sha256": cfg.hash(),
                "targets": cfg.targets.iter().map(|t| t.id.clone()).collect::<Vec<_>>(),
                "admin": if cfg.metrics_addr.is_empty() {
                    serde_json::json!("disabled")
                } else {
                    json!({"addr": cfg.metrics_addr, "auth": cfg.admin_token.is_some()})
                },
                "storage": serde_json::to_value(harvest::api::StorageHealth::from(&cfg))?,
                "last_run": harvest::api::recent_runs(&cfg.schedule.history_path, 1).pop(),
            });
            if live {
                let (addr, tok) = admin_target(&cfg, &url)?;
                v["diagnostics"] =
                    harvest::api::admin_get_json(&addr, "/diagnostics", tok.clone())?;
                v["stats"] = harvest::api::admin_get_json(&addr, "/stats", tok)?;
            }
            print_mode(&v, json, false, || {
                let mut out = String::new();
                let _ = std::fmt::Write::write_str(
                    &mut out,
                    &format!(
                        "run {} (policy sha {})\n",
                        cfg.name,
                        &cfg.hash()[..12.min(cfg.hash().len())]
                    ),
                );
                let r = &v["last_run"];
                if !r.is_null() {
                    let _ = writeln!(
                        out,
                        "last run: ts={} status={} written={}",
                        r["ts"],
                        r["status"].as_str().unwrap_or("?"),
                        r["written"]
                    );
                }
                let lanes = serde_json::to_value(harvest::api::StorageHealth::from(&cfg))
                    .unwrap_or_default();
                let mut rows = Vec::new();
                for (k, lane) in lanes.as_object().into_iter().flat_map(|m| m.iter()) {
                    rows.push(vec![
                        k.clone(),
                        lane["kind"].as_str().unwrap_or("?").to_string(),
                        lane["configured"]
                            .as_bool()
                            .map(|b| if b { "yes" } else { "no" })
                            .unwrap_or("?")
                            .to_string(),
                        lane["exists"]
                            .as_bool()
                            .map(|b| if b { "yes" } else { "no" })
                            .unwrap_or("?")
                            .to_string(),
                        lane["bytes"]
                            .as_u64()
                            .map(|b| b.to_string())
                            .unwrap_or_else(|| "-".into()),
                        lane["path"].as_str().unwrap_or("").to_string(),
                    ]);
                }
                let _ = write!(
                    out,
                    "{}",
                    harvest::api::render_table(
                        &["lane", "kind", "configured", "exists", "bytes", "path"],
                        &rows
                    )
                );
                if let Some(d) = v.get("diagnostics") {
                    if !d.is_null() {
                        let _ = writeln!(out, "diagnostics overall: {}", d["overall"]);
                        for comp in d["components"].as_array().into_iter().flatten() {
                            let _ = writeln!(out, "  {} = {}", comp["name"], comp["status"]);
                        }
                    }
                }
                out
            });
            Ok(())
        }
        Cmd::Health { config, url, json } => {
            let cfg = if config.exists() {
                Some(Config::load(&config).context("config invalid — health check FAILED")?)
            } else if url.is_some() {
                None
            } else {
                anyhow::bail!("no config at {} and no --url given", config.display());
            };
            // Live probe when an admin address is known.
            if url.is_some() || cfg.as_ref().is_some_and(|c| !c.metrics_addr.is_empty()) {
                let (addr, tok) =
                    admin_target(cfg.as_ref().unwrap_or(&harvest::Config::default()), &url)?;
                match harvest::api::AdminClient::new(&addr, tok).get("/health") {
                    Ok(reply) if reply.ok() => {
                        let body = reply.json().unwrap_or(serde_json::json!({"status": "ok"}));
                        print_mode(
                            &json!({"target": addr, "status": "healthy", "reply": body}),
                            json,
                            false,
                            || format!("{addr} healthy\n"),
                        );
                        return Ok(());
                    }
                    Ok(reply) => {
                        println!(
                            "{}",
                            json!({"target": addr, "status": "unhealthy", "code": reply.status})
                        );
                        anyhow::bail!("admin /health answered {}", reply.status);
                    }
                    Err(e) => {
                        println!(
                            "{}",
                            json!({"target": addr, "status": "unreachable", "detail": e.message})
                        );
                        anyhow::bail!("admin API unreachable at {addr}");
                    }
                }
            }
            // Offline: validated config + storage-lane presence. The lane
            // rules live in harvest::health (typed domain) — the CLI renders
            // them; unit + integration tests pin the vocabulary either way.
            let cfg = cfg.expect("checked above");
            let (checks, degraded) = harvest::health::lane_checks(&cfg);
            let overall = if degraded { "degraded" } else { "healthy" };
            let v = json!({"status": overall, "config": "ok", "checks": checks});
            print_mode(&v, json, false, || {
                format!(
                    "config OK — {overall}\n{}",
                    harvest::api::render_table(
                        &["lane", "status", "detail"],
                        &checks
                            .iter()
                            .map(|ck| vec![
                                ck["lane"].as_str().unwrap_or("?").to_string(),
                                ck["status"].as_str().unwrap_or("?").to_string(),
                                ck["detail"].as_str().unwrap_or("").to_string(),
                            ])
                            .collect::<Vec<_>>(),
                    )
                )
            });
            if degraded {
                anyhow::bail!("degraded: quarantine lane is not empty");
            }
            Ok(())
        }
        Cmd::Jobs {
            config,
            url,
            limit,
            json,
            jsonl,
        } => {
            let cfg = Config::load(&config)?;
            let v = match &url {
                Some(u) => {
                    let (addr, tok) = admin_target(&cfg, &Some(u.clone()))?;
                    harvest::api::admin_get_json(
                        &addr,
                        &format!("/jobs?limit={}", limit.clamp(1, 200)),
                        tok,
                    )?
                }
                None => {
                    let hist = harvest::api::history_window(&cfg.schedule.history_path, 1);
                    let last = hist.first();
                    let jobs: Vec<serde_json::Value> = last
                        .and_then(|h| h.pointer("/summary/per_target"))
                        .and_then(|m| m.as_object())
                        .map(|m| {
                            m.iter()
                                .map(|(tid, t)| {
                                    json!({"target": tid, "written": t["written"], "blocked": t["blocked"], "failed": t["failed"], "revalidated": t["revalidated"], "quarantined": t["quarantined"]})
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({
                        "source": "last persisted run (history)",
                        "run": cfg.name,
                        "jobs": jobs,
                    })
                }
            };
            let jobs = v["jobs"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .take(limit.clamp(1, 200))
                .collect::<Vec<_>>();
            print_mode(&json!({"jobs": jobs}), json, jsonl, || {
                let rows: Vec<Vec<String>> = jobs
                    .iter()
                    .map(|j| {
                        vec![
                            j["target"].as_str().unwrap_or("?").to_string(),
                            j["written"].to_string(),
                            j["blocked"].to_string(),
                            j["failed"].to_string(),
                            j["revalidated"].to_string(),
                            j["quarantined"].to_string(),
                        ]
                    })
                    .collect();
                harvest::api::render_table(
                    &[
                        "target",
                        "written",
                        "blocked",
                        "failed",
                        "reval",
                        "quarantined",
                    ],
                    &rows,
                )
            });
            Ok(())
        }
        Cmd::Runs {
            config,
            limit,
            offset,
            status,
            sort,
            order,
            since,
            until,
            json,
            jsonl,
        } => {
            let cfg = Config::load(&config)?;
            let qs = harvest::api::build_query(&[
                ("status", status),
                ("sort", sort),
                ("order", order),
                ("since", since.map(|v| v.to_string())),
                ("until", until.map(|v| v.to_string())),
            ]);
            let q = harvest::api::Query::parse(&qs).map_err(|e| anyhow::anyhow!(e.message))?;
            let filter =
                harvest::api::ListFilter::from_query(&q).map_err(|e| anyhow::anyhow!(e.message))?;
            let page = harvest::api::Page {
                limit: limit.clamp(1, 200),
                offset,
            };
            let (window, listed) =
                harvest::api::runs_listing(&cfg.schedule.history_path, &filter, page);
            let items = listed.items;
            print_mode(
                &json!({"window": window, "items": items}),
                json,
                jsonl,
                || {
                    let rows: Vec<Vec<String>> = items
                        .iter()
                        .map(|r| {
                            vec![
                                r.ts.to_string(),
                                r.run.clone(),
                                r.status.clone(),
                                r.written.to_string(),
                                r.quarantined.to_string(),
                                r.duration_ms.to_string(),
                                r.config_sha256
                                    .as_deref()
                                    .map(|s| s[..10.min(s.len())].to_string())
                                    .unwrap_or_default(),
                                r.error.clone().unwrap_or_default(),
                            ]
                        })
                        .collect();
                    harvest::api::render_table(
                        &[
                            "ts", "run", "status", "written", "quar", "ms", "policy", "error",
                        ],
                        &rows,
                    )
                },
            );
            Ok(())
        }
        Cmd::Workers { config, url, json } => {
            let cfg = Config::load(&config)?;
            let (addr, tok) = admin_target(&cfg, &url)?;
            let v = harvest::api::admin_get_json(&addr, "/workers", tok)?;
            print_mode(&v, json, false, || {
                let mut out = format!("workers state: {}\n", v["state"]);
                if let Some(s) = v.get("snapshot").filter(|s| !s.is_null()) {
                    let rows: Vec<Vec<String>> = s["workers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|w| {
                            vec![
                                w["id"].to_string(),
                                w["state"].as_str().unwrap_or("?").to_string(),
                                w["jobs_done"].to_string(),
                                w["jobs_failed"].to_string(),
                                w["restarts"].to_string(),
                                w["idle_secs"].to_string(),
                            ]
                        })
                        .collect();
                    out.push_str(&harvest::api::render_table(
                        &["id", "state", "done", "failed", "restarts", "idle_s"],
                        &rows,
                    ));
                }
                out
            });
            Ok(())
        }
        Cmd::Proxies { config, url, json } => {
            let cfg = Config::load(&config)?;
            let v = match &url {
                Some(u) => {
                    let (addr, tok) = admin_target(&cfg, &Some(u.clone()))?;
                    harvest::api::admin_get_json(&addr, "/proxies", tok)?
                }
                None => json!({
                    "source": "config (static view — live EWMA health needs a running admin API)",
                    "entries": cfg.proxies.iter().map(|p| json!({"id": p.id, "region": p.region})).collect::<Vec<_>>(),
                    "total": cfg.proxies.len(),
                }),
            };
            print_mode(&v, json, false, || {
                let rows: Vec<Vec<String>> = v["entries"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        vec![
                            e["id"].as_str().unwrap_or("?").to_string(),
                            e["region"].as_str().unwrap_or("direct").to_string(),
                            e["state"].as_str().unwrap_or("unprobed").to_string(),
                            e["requests"].to_string(),
                            e["failures"].to_string(),
                        ]
                    })
                    .collect();
                harvest::api::render_table(
                    &["id", "region", "state", "requests", "failures"],
                    &rows,
                )
            });
            Ok(())
        }
        #[cfg(not(feature = "redis"))]
        Cmd::Queue { config, url, json } => {
            let cfg = Config::load(&config)?;
            let (addr, tok) = admin_target(&cfg, &url)?;
            let v = harvest::api::admin_get_json(&addr, "/queue", tok)?;
            print_mode(&v, json, false, || {
                format!(
                    "queue state: {}\n{}",
                    v["state"],
                    match &v["stats"] {
                        serde_json::Value::Null => "no queue attached to the runtime\n".to_string(),
                        s => format!(
                            "mode={} depth={}/{} inflight={} pushed={} popped={} rejected={} expired={}\n",
                            s["mode"], s["depth"], s["max_depth"], s["inflight"],
                            s["pushed"], s["popped"], s["rejected"], s["expired"]
                        ),
                    }
                )
            });
            Ok(())
        }
        Cmd::Metrics { config, url, prom } => {
            let cfg = Config::load(&config)?;
            let (addr, tok) = admin_target(&cfg, &url)?;
            if prom {
                let reply = harvest::api::AdminClient::new(&addr, tok)
                    .get("/metrics")
                    .map_err(|e| anyhow::anyhow!(e.message))?;
                if !reply.ok() {
                    anyhow::bail!("admin /metrics answered {}", reply.status);
                }
                print!("{}", reply.body);
                return Ok(());
            }
            let v = harvest::api::admin_get_json(&addr, "/stats", tok)?;
            print_mode(&v, true, false, || {
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string())
            });
            Ok(())
        }
        Cmd::Datasets {
            config,
            dt,
            limit,
            json,
            jsonl,
        } => {
            let cfg = Config::load(&config)?;
            let limit = limit.clamp(1, 200);
            let (dir, is_dataset) = match &cfg.output {
                harvest::config::SinkCfg::Dataset { dir, .. } => (dir.clone(), true),
                other => {
                    // non-dataset sinks: report the single file lane
                    let v = serde_json::to_value(harvest::api::StorageHealth::from(&cfg))?
                        ["output"]
                        .clone();
                    let _ = other;
                    print_mode(
                        &json!({"note": "output sink is not a dataset directory", "output": v}),
                        json,
                        jsonl,
                        || format!("not a dataset sink: {}\n", v["kind"]),
                    );
                    return Ok(());
                }
            };
            let mut entries: Vec<std::path::PathBuf> = match std::fs::read_dir(&dir) {
                Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
                Err(_) => Vec::new(),
            };
            if let Some(dt) = &dt {
                // A full selector (`dt=YYYY-MM-DD`, bare, or compact) is
                // canonicalized through `ids` so `2026-01-31`,
                // `dt=2026-01-31`, and `20260131` all hit the same
                // partition; anything else keeps the documented
                // prefix/suffix convenience matching (`dt=2026`).
                let canon: Option<String> = harvest::ids::DatasetPartition::parse_selector(dt)
                    .ok()
                    .map(|s| s.dir_name());
                entries.retain(|p| {
                    let n = p
                        .file_name()
                        .map(|x| x.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    match &canon {
                        Some(label) => n.starts_with(label) || n.ends_with(label) || &n == label,
                        None => n.starts_with(dt.as_str()) || n.ends_with(dt.as_str()),
                    }
                });
            }
            entries.sort();
            let parts: Vec<serde_json::Value> = entries
                .iter()
                .take(limit)
                .map(|p| {
                    let files = std::fs::read_dir(p)
                        .map(|rd| rd.flatten().count())
                        .unwrap_or(0);
                    let bytes = if p.is_dir() {
                        harvest::api::dir_size_bounded(p, 4096)
                    } else {
                        p.metadata().map(|m| m.len()).unwrap_or(0)
                    };
                    json!({
                        "name": p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                        "parts": files,
                        "bytes": bytes,
                        "mtime_unix": p.metadata().ok().and_then(|m| m.modified().ok())
                            .map(|t| t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
                            .unwrap_or(0),
                    })
                })
                .collect();
            let v = json!({
                "dir": harvest::api::path_label(&dir),
                "dataset": is_dataset,
                "partitions": parts.len(),
                "items": parts,
            });
            print_mode(&v, json, jsonl, || {
                let rows: Vec<Vec<String>> = v["items"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .map(|e| {
                        vec![
                            e["name"].as_str().unwrap_or("?").to_string(),
                            e["parts"].to_string(),
                            e["bytes"].to_string(),
                            e["mtime_unix"].to_string(),
                        ]
                    })
                    .collect();
                harvest::api::render_table(&["partition", "parts", "bytes", "mtime"], &rows)
            });
            Ok(())
        }
        Cmd::Config { cmd } => match cmd {
            ConfigCmd::Inspect { config, json } => {
                let cfg = Config::load(&config)?;
                let v = harvest::api::config_inspect(&cfg, Some(&config));
                print_mode(&v, json, false, || {
                    let mut rows = Vec::new();
                    if let Some(obj) = v.as_object() {
                        let sources = obj.get("_sources").cloned().unwrap_or(json!({}));
                        for (k, val) in obj.iter() {
                            if k.starts_with('_')
                                || k == "backends"
                                || k == "validation"
                                || k == "config_sha256"
                            {
                                continue;
                            }
                            rows.push(vec![
                                k.clone(),
                                sources[k].as_str().unwrap_or("?").to_string(),
                                val.to_string().chars().take(60).collect(),
                            ]);
                        }
                    }
                    format!(
                        "validation: {} | sha256 {} | backends {}\n{}",
                        v["validation"],
                        v["config_sha256"]
                            .as_str()
                            .unwrap_or("-")
                            .get(..12)
                            .unwrap_or("-"),
                        v["backends"],
                        harvest::api::render_table(&["key", "source", "value"], &rows)
                    )
                });
                Ok(())
            }
            ConfigCmd::Migrate {
                config,
                out,
                dry_run,
                json,
            } => {
                // Operate on the file AS WRITTEN: env references must not be
                // expanded into the output document.
                let raw = std::fs::read_to_string(&config)
                    .with_context(|| format!("cannot read config {}", config.display()))?;
                let value: serde_json::Value =
                    serde_json::from_str(&raw).context("config is not valid JSON")?;
                harvest::migrate::check_unknown_keys(&value, Config::TOP_LEVEL_KEYS)
                    .map_err(anyhow::Error::msg)?;
                let declared = value
                    .get("config_version")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32)
                    .unwrap_or(1);
                let (doc, warnings) =
                    harvest::migrate::migrate(&value).map_err(anyhow::Error::msg)?;
                let written = match (&out, dry_run) {
                    (Some(p), false) => {
                        let body = format!(
                            "{}\n",
                            serde_json::to_string_pretty(&doc).context("serialize")?
                        );
                        std::fs::write(p, body)
                            .with_context(|| format!("write {}", p.display()))?;
                        Some(p.clone())
                    }
                    _ => None,
                };
                let v = json!({
                    "from": declared,
                    "to": harvest::migrate::CURRENT_VERSION,
                    "actions": warnings,
                    "written": written.as_ref().map(|p| harvest::api::path_label(p)),
                });
                print_mode(&v, json, false, || {
                    let mut s = String::new();
                    let _ = std::fmt::Write::write_str(
                        &mut s,
                        &format!(
                            "config_version: {declared} -> {}\n",
                            harvest::migrate::CURRENT_VERSION
                        ),
                    );
                    for w in &warnings {
                        let _ = writeln!(s, "  note: {w}");
                    }
                    match &written {
                        Some(p) => {
                            let _ = writeln!(s, "migrated config written to {}", p.display());
                        }
                        None => {
                            let _ = writeln!(s, "dry run — nothing written (use --out FILE)");
                        }
                    }
                    s
                });
                Ok(())
            }
            ConfigCmd::Explain {
                config,
                field,
                json,
            } => {
                let (cfgv, sources) = if config.exists() {
                    let cfg = Config::load(&config)?;
                    let full = harvest::api::config_inspect(&cfg, Some(&config));
                    let src = full.get("_sources").cloned();
                    (full, src)
                } else {
                    (json!({}), None)
                };
                let rows = harvest::api::explain_config(&cfgv, sources.as_ref(), field.as_deref());
                print_mode(&json!({"knobs": rows}), json, false, || {
                    let table: Vec<Vec<String>> = rows
                        .iter()
                        .map(|r| {
                            vec![
                                r["path"].as_str().unwrap_or("?").to_string(),
                                r["type"].as_str().unwrap_or("?").to_string(),
                                r["default"].as_str().unwrap_or("?").to_string(),
                                r.get("current")
                                    .map(|v| v.to_string())
                                    .unwrap_or_else(|| "-".into()),
                                r["summary"].as_str().unwrap_or("?").to_string(),
                            ]
                        })
                        .collect();
                    harvest::api::render_table(
                        &["path", "type", "default", "current", "summary"],
                        &table,
                    )
                });
                Ok(())
            }
        },
        Cmd::Audit { file, md } => {
            let txt = std::fs::read_to_string(&file)
                .with_context(|| format!("read {}", file.display()))?;
            let v: serde_json::Value = serde_json::from_str(&txt)?;
            if md {
                println!("{}", harvest::audit::render_markdown(&v));
            } else {
                println!("{}", serde_json::to_string_pretty(&v)?);
            }
            Ok(())
        }
        Cmd::Export {
            from,
            to,
            split_target,
            history,
            key,
            max_part_bytes,
            resume,
            format,
            dataset,
        } => {
            let formats = harvest::export::parse_formats(&format)?;
            let r = harvest::export::run(&harvest::export::ExportOpts {
                inputs: from,
                out_dir: to,
                split_target,
                history,
                key,
                max_part_bytes,
                resume,
                formats,
                dataset,
                pdf_sample_rows: 12,
            })?;
            println!("{}", serde_json::to_string_pretty(&r.manifest)?);
            println!("formats: {}", r.formats.join(", "));
            if let Some(b) = &r.bundle {
                println!(
                    "bundle: {} ({} bytes, {} members, sha256 {})",
                    std::path::Path::new(&r.out_dir).join(&b.name).display(),
                    b.bytes,
                    b.members,
                    b.sha256
                );
            }
            if let Some(s) = &r.manifest_sig {
                println!(
                    "signed: {s} (verify with `harvest attest --verify <manifest> --key <key>`)"
                );
            }
            Ok(())
        }
        Cmd::Verify { dir, key, json } => {
            let v = harvest::export::verify(&dir, key.as_deref())?;
            let body = serde_json::to_value(&v)?;
            print_mode(&body, json, false, || {
                let mut s = String::new();
                if let Some(e) = &v.error {
                    s.push_str(&format!("FAIL: {e}\n"));
                    return s;
                }
                let _ = writeln!(
                    s,
                    "verify {} — {} file(s), {} record(s)",
                    dir.display(),
                    v.files.len(),
                    v.records
                );
                for f in &v.files {
                    let _ = writeln!(
                        s,
                        "  {:<40} present={:<5} bytes={:<5} sha={:<5} lines={:<5}",
                        f.name, f.present, f.bytes_ok, f.sha_ok, f.lines_ok
                    );
                }
                for x in &v.extras {
                    let _ = writeln!(s, "  extra (not in manifest): {x}");
                }
                let _ = writeln!(
                    s,
                    "  records total: {}",
                    if v.records_ok { "ok" } else { "MISMATCH" }
                );
                let _ = writeln!(s, "  signature: {}", v.signature);
                let _ = writeln!(s, "verdict: {}", if v.ok { "PASS" } else { "FAIL" });
                s
            });
            if v.ok {
                Ok(())
            } else {
                anyhow::bail!("export failed verification — see report above")
            }
        }
        Cmd::Attest {
            gen_key,
            verify,
            sig,
            key,
        } => match (gen_key, verify) {
            (Some(g), None) => {
                harvest::sign::Key::generate(&g)?;
                println!(
                    "wrote 32-byte HMAC key (mode 0600) to {} — set \"attest_key_path\" \
                         in the config to that file; share the key only with verifiers",
                    g.display()
                );
                Ok(())
            }
            (None, Some(file)) => {
                let key = key.context("--key <file> is required for verification")?;
                let sigp = sig.unwrap_or_else(|| harvest::sign::sig_path(&file));
                let k = harvest::sign::Key::load(&key)?;
                k.verify_file(&file, &sigp).with_context(|| {
                    format!("verify {} against {}", file.display(), sigp.display())
                })?;
                println!(
                    "OK: {} is intact and was signed by a holder of {}",
                    file.display(),
                    key.display()
                );
                Ok(())
            }
            _ => anyhow::bail!(
                "use either `attest --gen-key <path>` or `attest --verify <file> --key <path>`"
            ),
        },
        #[cfg(feature = "redis")]
        #[cfg(feature = "redis")]
        Cmd::Queue {
            config,
            url,
            json,
            redis,
            prefix,
        } => {
            let cfg = Config::load(&config)?;
            // Distributed lane first (legacy output, byte-identical):
            // `--redis`/`redis_url` present => redis counters, not admin.
            if let Some(rurl) = redis.or_else(|| cfg.redis_url.clone()) {
                let mut q = harvest::dist::Queue::connect(&rurl, &prefix)
                    .await
                    .with_context(|| {
                        format!("redis {}", harvest::config::redact_credentials(&rurl))
                    })?;
                println!("{}", serde_json::to_string_pretty(&q.counts().await?)?);
                return Ok(());
            }
            let (addr, tok) = admin_target(&cfg, &url)?;
            let v = harvest::api::admin_get_json(&addr, "/queue", tok)?;
            print_mode(&v, json, false, || {
                format!(
                    "queue state: {}\n{}",
                    v["state"],
                    match &v["stats"] {
                        serde_json::Value::Null => "no queue attached to the runtime\n".to_string(),
                        s => format!(
                            "mode={} depth={}/{} inflight={} pushed={} popped={} rejected={} expired={}\n",
                            s["mode"], s["depth"], s["max_depth"], s["inflight"],
                            s["pushed"], s["popped"], s["rejected"], s["expired"]
                        ),
                    }
                )
            });
            Ok(())
        }
        Cmd::Validate { config } => {
            let cfg = Config::load(&config)?;
            println!(
                "config OK — {} target(s), {} proxy egress(es), policy sha256 {}",
                cfg.targets.len(),
                cfg.proxies.len().max(1),
                cfg.hash()
            );
            // Soft layer: migration notes + production audit. Warnings
            // never change the exit code — hard rules are `validate`'s job.
            for w in cfg
                .load_warnings
                .iter()
                .chain(cfg.production_audit().iter())
            {
                println!("warning: {w}");
            }
            for t in &cfg.targets {
                println!(
                    "  target `{}`: {} fields (mode={}, sitemap={}), follow_links={}, max_depth={}",
                    t.id,
                    t.extract.len(),
                    t.mode,
                    t.sitemap,
                    t.follow_links,
                    t.max_depth
                );
            }
            println!(
                "policy: respect_robots={} default {} rps/host, block backoff {}s, breaker at {} blocks",
                cfg.respect_robots, cfg.default_policy.requests_per_sec, cfg.block_backoff_secs, cfg.block_breaker_count
            );
            println!(
                "schedule: every {}s, runs {} (0 = until stopped), history {}",
                cfg.schedule.every_secs,
                cfg.schedule.runs,
                cfg.schedule.history_path.display()
            );
            println!(
                "revalidation: {}",
                match &cfg.revalidate_store {
                    Some(p) => format!(
                        "{} (conditional GET{} across runs)",
                        p.display(),
                        if cfg.revalidate_ttl_secs > 0 {
                            format!(", entries age out after {}s", cfg.revalidate_ttl_secs)
                        } else {
                            String::new()
                        }
                    ),
                    None => "disabled (set revalidate_store for cheap repeat crawls)".into(),
                }
            );
            println!(
                "checkpoint: {}",
                match &cfg.ckpt_path {
                    Some(p) => format!(
                        "{} (every {} pages or {}s)",
                        p.display(),
                        cfg.ckpt_every_pages,
                        cfg.ckpt_every_secs
                    ),
                    None => "disabled (set ckpt_path for resumable crawls)".into(),
                }
            );
            println!(
                "admin: {} | alerts: {}",
                if cfg.admin_token.is_some() {
                    "bearer token required"
                } else {
                    "open (bind to localhost/internal only)"
                },
                if cfg.alerts.enabled() {
                    format!(
                        "webhook {} events {:?}",
                        cfg.alerts
                            .webhook_url
                            .as_deref()
                            .map(harvest::config::redact_url_origin)
                            .unwrap_or_default(),
                        cfg.alerts.events.join(",")
                    )
                } else {
                    "disabled".into()
                },
            );
            Ok(())
        }
        Cmd::Schema { config, target } => {
            let cfg = Config::load(&config)?;
            let mut printed = 0usize;
            for t in &cfg.targets {
                if let Some(want) = &target {
                    if want != &t.id {
                        continue;
                    }
                }
                let doc = harvest::schema::json_schema_for(t);
                // One JSON doc per target: `jq -s` friendly, and
                // --target prints exactly one.
                println!("{}", serde_json::to_string(&doc)?);
                printed += 1;
            }
            if printed == 0 {
                if let Some(want) = &target {
                    anyhow::bail!("no target `{want}` in {}", config.display());
                }
                anyhow::bail!("config has no targets");
            }
            Ok(())
        }
        Cmd::Run {
            config,
            max_pages,
            retry_dlq,
            resume,
        } => {
            let mut cfg = Config::load(&config)?;
            if let Some(mp) = max_pages {
                cfg.max_pages = mp;
            }
            // Graceful drain: ctrl-c stops link discovery + queueing, workers
            // finish in-flight requests, partial results are flushed to disk.
            let stop = Arc::new(AtomicBool::new(false));
            let stop_sig = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::warn!(
                    "stop requested — finishing in-flight requests, then writing results"
                );
                stop_sig.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let outcome =
                harvest::engine::run_with_stop(cfg.clone(), Some(stop.clone()), retry_dlq, resume)
                    .await;
            let (summary, status) = match &outcome {
                Ok(s) => (
                    s.clone(),
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        "cancelled"
                    } else {
                        "ok"
                    },
                ),
                Err(_) => (harvest::engine::Summary::default(), "error"),
            };
            let mut entry = harvest::runner::history_entry(&cfg, &summary, status);
            if let Err(e) = &outcome {
                entry["error"] = e.to_string().into();
            }
            if let Err(e) = harvest::runner::record_history(&cfg, &entry) {
                tracing::warn!(err = %e, "could not write run history");
            }
            harvest::alerts::check_and_alert(&cfg, status, &summary).await;
            let summary = outcome?;
            println!("{}", serde_json::to_string_pretty(&summary)?);
            println!(
                "history: {}  config sha256 {}",
                cfg.schedule.history_path.display(),
                cfg.hash()
            );
            Ok(())
        }
        Cmd::Serve {
            config,
            every_secs,
            runs,
        } => {
            let mut cfg = Config::load(&config)?;
            if let Some(e) = every_secs {
                cfg.schedule.every_secs = e;
            }
            if let Some(r) = runs {
                cfg.schedule.runs = r;
            }
            cfg.validate().context("config invalid after overrides")?;
            tracing::info!(
                run = %cfg.name,
                every_secs = cfg.schedule.every_secs,
                runs = cfg.schedule.runs,
                dashboard = %cfg.metrics_addr,
                "scheduler started (ctrl-c stops after the current run)"
            );
            let stop = Arc::new(AtomicBool::new(false));
            let stop_sig = stop.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                tracing::warn!("stop requested — finishing current run, then exiting");
                stop_sig.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            let done = harvest::runner::serve(cfg, stop).await?;
            println!("scheduler stopped after {done} run(s)");
            Ok(())
        }
        Cmd::History { config, n } => {
            let cfg = Config::load(&config)?;
            let txt = std::fs::read_to_string(&cfg.schedule.history_path).unwrap_or_default();
            let lines: Vec<&str> = txt.lines().collect();
            if lines.is_empty() {
                println!("no history yet at {}", cfg.schedule.history_path.display());
                return Ok(());
            }
            for line in lines.iter().rev().take(n).rev() {
                let v: serde_json::Value = serde_json::from_str(line)
                    .unwrap_or_else(|_| serde_json::json!({ "raw": line }));
                println!("{}", serde_json::to_string(&v)?);
            }
            Ok(())
        }
        Cmd::Fetch { url } => {
            let u = Url::parse(&url)?;
            let cfg = Arc::new(Config::default());
            let pool = Arc::new(harvest::proxy::ProxyPool::new(&cfg.proxies));
            let metrics = Arc::new(harvest::metrics::Metrics::default());
            let fetcher = harvest::fetcher::Fetcher::new(cfg, pool);
            let o = fetcher
                .get(u, &metrics)
                .await
                .map_err(|e| anyhow::anyhow!("{e:?}"))?;
            println!(
                "{} {} bytes {} ct={} egress={}",
                o.status, o.url, o.bytes, o.content_type, o.proxy_id
            );
            Ok(())
        }
    }
}
