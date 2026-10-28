use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Maximum body size for one HTTP response. Larger values are rejected before
/// a run can allocate an unbounded per-fetch buffer.
pub const MAX_RESPONSE_BYTES_CEILING: u64 = 64 * 1024 * 1024;
/// Conservative aggregate reservation for simultaneously buffered responses.
pub const MAX_IN_FLIGHT_RESPONSE_BYTES: u64 = 1024 * 1024 * 1024;

/// Whole-engine configuration. Loaded from a single JSON file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Schema stamp — see [`crate::migrate`]. Absent in a file means v1
    /// (the pre-versioning baseline); loads migrate forward automatically
    /// and refuse anything stamped newer than this build understands.
    #[serde(default = "crate::migrate::current_version_default")]
    pub config_version: u32,
    /// Soft warnings produced while loading/migrating this file (never
    /// errors — `validate` still fails loudly). Skipped in serialization:
    /// load-time metadata, not config content.
    #[serde(default, skip)]
    pub load_warnings: Vec<String>,
    pub name: String,
    /// Identify your crawler honestly. Enterprise data collection is usually
    /// whitelisted by name; a fake "Mozilla" UA is neither legal nor effective.
    pub user_agent: String,
    /// Hard refusal switch: if true (default), robots.txt is mandatory.
    /// Setting this to false only applies to sources you have written
    /// permission to collect from; the engine will still honor rate limits.
    pub respect_robots: bool,

    pub concurrency: usize,
    pub request_timeout_secs: u64,
    pub connect_timeout_secs: u64,
    /// Transport/5xx retries per request.
    pub max_retries: u32,
    pub retry_base_ms: u64,
    /// Never sleep longer than this inside a request retry loop.
    pub max_single_wait_ms: u64,
    /// Hard ceiling for a single response body. The content-length header is
    /// checked before streaming, and the stream is capped while reading, so a
    /// hostile or misconfigured server cannot make the engine buffer an
    /// unbounded body into memory. Anything over this fails the URL
    /// permanently (no retries: the answer will not change).
    pub max_response_bytes: u64,

    /// Global page budget for a run (all targets combined).
    pub max_pages: usize,

    pub default_policy: DomainPolicy,
    /// Per-domain overrides keyed by registrable host, e.g. "example.com".
    pub domain_policies: BTreeMap<String, DomainPolicy>,

    /// Seconds of backoff applied to a domain after an HTTP block (401/403/
    /// 429/451); doubles per consecutive block, capped at 1 h.
    pub block_backoff_secs: u64,
    /// After this many blocks in one run, the domain is abandoned entirely.
    pub block_breaker_count: u32,
    pub robots_ttl_secs: u64,

    /// Optional dead-letter queue file: transport failures and honored
    /// refusals are appended here; `run --retry-dlq` / `push --from-dlq`
    /// re-seed them with attempt accounting.
    /// Simultaneous requests through ONE proxy egress (`0` = unlimited).
    /// An attempt that would exceed the cap is refused as a retryable
    /// transport failure, so the retry/backoff machinery supplies the wait.
    #[serde(default)]
    pub max_inflight_per_proxy: u64,
    /// Hard cap on records WRITTEN by a single run (`0` = unlimited). At
    /// the cap the scheduler drains gracefully — in-flight jobs finish,
    /// queued work is abandoned, checkpoint/audit/exit stay clean — so a
    /// capped run is a successful run with `written == limit`.
    #[serde(default)]
    pub max_records_per_run: u64,
    pub dlq_path: Option<PathBuf>,
    /// Requeue cap per URL before it stays dead for good (default 3).
    pub dlq_max_attempts: u32,
    /// Redis shared-state endpoint for queue hand-off (`harvest push/worker`).
    pub redis_url: Option<String>,
    /// Checkpoint file for pause/resume of long crawls (`run --resume`).
    /// The engine snapshots queue + visited + fingerprints here every
    /// `ckpt_every_pages` records or `ckpt_every_secs` seconds (whichever
    /// first); a completed run deletes it. None disables checkpointing.
    #[serde(default)]
    pub ckpt_path: Option<PathBuf>,
    #[serde(default = "default_ckpt_secs")]
    pub ckpt_every_secs: u64,
    #[serde(default = "default_ckpt_pages")]
    pub ckpt_every_pages: usize,
    /// Incremental revalidation store (ETag/Last-Modified + cached link
    /// graph). Set to enable conditional GETs across runs — repeat crawls
    /// pay 304s, not bodies. None disables the feature.
    #[serde(default)]
    pub revalidate_store: Option<PathBuf>,
    /// Re-check validators only if younger than this; 0 = never expire.
    #[serde(default)]
    pub revalidate_ttl_secs: u64,
    /// Politeness audit report path: per-host request/pacing/robots receipt
    /// written at run end (JSON; `harvest audit --md` renders it). None = off.
    #[serde(default)]
    pub audit_path: Option<PathBuf>,
    /// Structured-event log: one JSON object per line — job lifecycle,
    /// request/attempt/proxy/retry, extraction and sink completions,
    /// queue/drain/breaker transitions. `None` disables the file; the
    /// in-memory event ring still feeds `GET /events` for live tailing.
    #[serde(default)]
    pub events_path: Option<PathBuf>,
    /// Deterministic retry backoff (jitter forced to 0) — unit/integration
    /// tests assert exact delays with this on; production leaves it false.
    #[serde(default)]
    pub retry_deterministic: bool,
    /// Engine-layer scheduling policy (queue mode, bounds, restarts,
    /// deadlines, retry budget). All defaults reproduce the pre-scheduler
    /// behavior exactly; every feature is opt-in.
    #[serde(default)]
    pub scheduling: SchedulingCfg,
    /// Per-host transport circuit breaker (connection refused/reset/timeout
    /// and 5xx — NOT bot blocks, which the politeness layer owns). Unset
    /// disables the breaker entirely.
    #[serde(default)]
    pub transport_breaker: Option<BreakerOpts>,
    /// HMAC key file for signing delivery evidence (`audit.json.sig`,
    /// `manifest.json.sig`). Set => every artifact gets a detached signature;
    /// a missing/unreadable key fails the run at startup, not after the crawl.
    #[serde(default)]
    pub attest_key_path: Option<PathBuf>,

    /// When set, every admin endpoint (incl. /metrics) requires
    /// `Authorization: Bearer <token>`. Leave unset on a trusted localhost /
    /// internal network; set it the moment the port can be routed to.
    pub admin_token: Option<String>,
    /// SLO alerts evaluated after every run (both `run` and `serve`).
    pub alerts: AlertCfg,

    /// Opt-in W3C WebDriver runtime. HTTP-only remains the default transport.
    #[serde(default)]
    pub browser: crate::browser::config::BrowserConfig,
    pub proxies: Vec<ProxyCfg>,
    pub targets: Vec<TargetCfg>,
    pub output: SinkCfg,
    pub metrics_addr: String,

    /// Cadence settings for `harvest serve` (one-shot `run` ignores them).
    pub schedule: ScheduleCfg,

    /// Raw-file sha256, filled by `Config::load`; embedded into every run
    /// history entry so a client can later prove *which* policy ran.
    #[serde(skip)]
    pub source_hash: Option<String>,
    /// Where the config was loaded from, when it was loaded from a file.
    /// Used only by `/config` to diff explicit vs default values; never
    /// serialized, hashed, or sent to clients.
    #[serde(skip)]
    pub source_path: Option<std::path::PathBuf>,
}

/// Per-run SLO alerts. Fires a webhook POST with the offending reasons when
/// thresholds are breached. `webhook_url` unset (or empty) disables alerts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AlertCfg {
    pub webhook_url: Option<String>,
    /// Which conditions fire. Supported: "run_error", "no_data", "high_block_rate".
    pub events: Vec<String>,
    /// no_data threshold: alert when written records < this (0 = alert at zero).
    pub min_records: u64,
    /// high_block_rate threshold: blocked/requests ratio; 0 disables the check.
    pub max_block_rate: f64,
    /// Per-target SLOs, keyed by target id. Checked with the run's
    /// per-target summary; fires `<event>:target=<id>:<detail>` reasons.
    pub per_target: BTreeMap<String, TargetSlo>,
    /// quarantine_surge event: fires when a run quarantines at least this
    /// many records. 0 disables the check (default — the event never fired
    /// before this knob existed).
    #[serde(default)]
    pub quarantine_threshold: u64,
    /// Minimum interval between webhook deliveries for the same event
    /// (dedup/cooldown). 0 = notify every firing (legacy behavior).
    #[serde(default)]
    pub cooldown_secs: u64,
    /// Alert severity floor for webhook delivery: "info" | "warning" |
    /// "error". Lower-severity events still fire (visible in the ledger and
    /// metrics) but never page. Unknown values validate to "info".
    #[serde(default)]
    pub min_severity: String,
}

impl AlertCfg {
    pub fn enabled(&self) -> bool {
        matches!(&self.webhook_url, Some(u) if !u.is_empty()) && !self.events.is_empty()
    }
    pub fn wants(&self, event: &str) -> bool {
        self.events.iter().any(|e| e == event)
    }
}

/// SLO thresholds for a single target within a run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TargetSlo {
    /// Alert when records written < this (0 = alert only at zero).
    pub min_records: u64,
    /// blocked / (written + blocked + failed) ratio that alerts; 0 disables.
    pub max_block_rate: f64,
}

/// Scheduler policy for repeated runs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScheduleCfg {
    /// Seconds between run starts. 0 = start the next run immediately.
    pub every_secs: u64,
    /// Total runs to execute; 0 = until stopped (ctrl-c).
    pub runs: usize,
    /// JSONL run-history log (also the data source for the web dashboard).
    pub history_path: PathBuf,
}

impl Default for ScheduleCfg {
    fn default() -> Self {
        Self {
            every_secs: 3600,
            runs: 0,
            history_path: PathBuf::from("out/runs.jsonl"),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            config_version: crate::migrate::CURRENT_VERSION,
            load_warnings: Vec::new(),
            name: "harvest".into(),
            user_agent: "harvest/0.1 (compliant data engine; contact: ops@example.com)".into(),
            respect_robots: true,
            concurrency: 4,
            request_timeout_secs: 20,
            connect_timeout_secs: 10,
            max_retries: 3,
            retry_base_ms: 500,
            max_single_wait_ms: 15_000,
            max_response_bytes: 16 * 1024 * 1024,
            max_pages: 500,
            default_policy: DomainPolicy::default(),
            domain_policies: BTreeMap::new(),
            block_backoff_secs: 60,
            block_breaker_count: 5,
            robots_ttl_secs: 3600,
            max_inflight_per_proxy: 0,
            max_records_per_run: 0,
            dlq_path: None,
            dlq_max_attempts: 3,
            ckpt_path: None,
            events_path: None,
            retry_deterministic: false,
            scheduling: SchedulingCfg::default(),
            transport_breaker: None,
            ckpt_every_secs: default_ckpt_secs(),
            ckpt_every_pages: default_ckpt_pages(),
            revalidate_store: None,
            revalidate_ttl_secs: 0,
            audit_path: None,
            attest_key_path: None,
            redis_url: None,
            admin_token: None,
            alerts: AlertCfg::default(),
            browser: crate::browser::config::BrowserConfig::default(),
            proxies: Vec::new(),
            targets: Vec::new(),
            output: SinkCfg::default(),
            metrics_addr: "127.0.0.1:9678".into(),
            schedule: ScheduleCfg::default(),
            source_hash: None,
            source_path: None,
        }
    }
}

/// Engine-layer scheduling policy. Every field defaults to the exact
/// behavior the engine had before this module existed, so opting in is
/// strictly additive.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchedulingCfg {
    /// `"fifo"` (default, legacy-exact), `"priority"` (High before Normal
    /// before Low, FIFO within a class), or `"fair"` (round-robin across
    /// targets so one big site cannot starve the others).
    pub mode: String,
    /// Bounded queue depth across the whole run; 0 = unlimited. When the
    /// cap is reached, discovery stops accepting new URLs (counted as
    /// `queue_rejected` in /stats — that counter *is* the backpressure
    /// signal). Nothing already queued is ever evicted.
    pub max_queue_depth: u32,
    /// fair mode: max jobs executing concurrently per target. 0 = no
    /// separate limit (engine concurrency still caps the total).
    pub per_target_inflight: u32,
    /// Restart attempts granted to a worker task that dies by panic.
    /// 0 = legacy semantics: the first death ends the run.
    pub worker_restarts: u32,
    /// Max age of a queued job before it expires unread. 0 = no deadline
    /// (a job waits forever for its turn).
    pub job_max_age_secs: u64,
    /// Global retries per minute the retry policy may grant (token bucket
    /// shared by all workers). 0 = unlimited.
    pub retry_budget_per_min: u32,
}

impl Default for SchedulingCfg {
    fn default() -> Self {
        Self {
            mode: "fifo".into(),
            max_queue_depth: 0,
            per_target_inflight: 0,
            worker_restarts: 0,
            job_max_age_secs: 0,
            retry_budget_per_min: 0,
        }
    }
}

/// Transport circuit breaker (per host): after `min_consecutive`
/// transport/5xx failures, jobs for that host are skipped (failed to the
/// DLQ path) for `cooldown_secs`, then one probe request tests recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BreakerOpts {
    pub min_consecutive: u32,
    pub cooldown_secs: u64,
    /// Probe successes (in a row) required to close the circuit.
    pub recovery_successes: u32,
}

impl Default for BreakerOpts {
    fn default() -> Self {
        Self {
            min_consecutive: 3,
            cooldown_secs: 60,
            recovery_successes: 1,
        }
    }
}

/// Per-domain pacing. Enforced through a token bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DomainPolicy {
    /// Sustained requests per second to a single host.
    pub requests_per_sec: f64,
    /// Burst capacity of the bucket.
    pub burst: u32,
    /// Absolute minimum gap between two calls to the same host.
    pub min_interval_ms: u64,
}

impl Default for DomainPolicy {
    fn default() -> Self {
        Self {
            requests_per_sec: 1.0,
            burst: 2,
            min_interval_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProxyCfg {
    pub id: String,
    /// e.g. "http://user:pass@fra-1.gateway.internal:8000" or "socks5h://...".
    /// `None` means a direct egress.
    pub url: Option<String>,
    pub region: Option<String>,
}

impl Default for ProxyCfg {
    fn default() -> Self {
        Self {
            id: "direct".into(),
            url: None,
            region: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TargetCfg {
    pub id: String,
    pub start_url: String,
    /// How many levels of same-host links to follow (0 = start URL only).
    pub max_depth: usize,
    pub follow_links: bool,
    /// CSS selector used to discover links when `follow_links` is true.
    pub link_selector: Option<String>,
    /// Only follow URLs whose href contains one of these substrings (empty = all).
    pub include: Vec<String>,
    /// Skip URLs whose href contains any of these substrings.
    pub exclude: Vec<String>,
    /// Field name -> extraction rule.
    pub extract: BTreeMap<String, FieldCfg>,
    /// Per-target page budget (also bounded by global max_pages).
    pub max_pages: usize,
    /// Extraction mode: "html" (CSS selectors, default) or "json"
    /// (JSONPath-lite over application/json bodies — API sources).
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Seed this target's queue from the site's sitemap (RFC-9309 §6.2
    /// style discovery): `<origin>/sitemap.xml` unless `sitemap_url` is set.
    /// Child `<sitemapindex>` entries are fetched too. Failures degrade to
    /// plain start-URL crawling with a warning, never abort the run.
    #[serde(default)]
    pub sitemap: bool,
    #[serde(default)]
    pub sitemap_url: Option<String>,
    /// Record schema policy (transforms pipeline still applies; this is the
    /// validation layer on top). Default `off` = no schema enforcement.
    pub schema: SchemaCfg,
    /// Opt-in browser transport. If false, the existing HTTP fetcher is used.
    #[serde(default)]
    pub browser: crate::browser::config::TargetBrowserConfig,
    /// Optional hierarchical selector tree; legacy `extract` rules remain supported.
    #[serde(default)]
    pub selector_tree: Option<crate::selector::ast::SelectorTree>,
}

fn default_mode() -> String {
    "html".into()
}

impl Default for TargetCfg {
    fn default() -> Self {
        Self {
            id: "target".into(),
            start_url: String::new(),
            max_depth: 0,
            follow_links: false,
            link_selector: None,
            include: Vec::new(),
            exclude: Vec::new(),
            extract: BTreeMap::new(),
            max_pages: 20,
            mode: default_mode(),
            sitemap: false,
            sitemap_url: None,
            schema: SchemaCfg::default(),
            browser: crate::browser::config::TargetBrowserConfig::default(),
            selector_tree: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldCfg {
    /// CSS selector, e.g. "h1", "span.price", "a.next". (html mode)
    #[serde(default)]
    pub css: String,
    /// JSONPath-lite query, e.g. "$.data.items[*].name". (json mode)
    #[serde(default)]
    pub path: Option<String>,
    /// Attribute to read instead of text, e.g. "href", "content".
    #[serde(default)]
    pub attr: Option<String>,
    /// Collect every match as an array (default: first match only).
    #[serde(default)]
    pub all: bool,
    /// Ordered value transforms applied after extraction, before the record
    /// is written (see `transform::Pipeline`). Empty = pass-through — the
    /// default, and byte-identical to pre-pipeline behavior.
    #[serde(default)]
    pub transforms: Vec<TransformCfg>,
}

/// One transform step, compiled once per run by `transform::Pipeline`.
/// String steps map over array fields element-wise; list steps require an
/// array (or a string for `split`). A failing step keeps the original
/// value and records the reason — which becomes a quarantine decision when
/// the target enforces its schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum TransformCfg {
    Trim,
    Lowercase,
    Uppercase,
    /// Collapse internal whitespace runs to one space.
    CollapseWhitespace,
    RegexReplace {
        pattern: String,
        replacement: String,
    },
    /// Keep only the first match (capture group `group`, default 0).
    RegexExtract {
        pattern: String,
        #[serde(default)]
        group: usize,
    },
    /// Remove HTML tags + decode the five named entities plus numeric refs.
    StripTags,
    /// Email / phone / Luhn-validated card numbers / IPv4 — masked per
    /// `strategy` (`kinds`: subsets of email|phone|card|ipv4; all when
    /// omitted). Collection *redaction*, not detection: what you store.
    MaskPii {
        #[serde(default)]
        strategy: MaskStrategy,
        #[serde(default)]
        kinds: Vec<String>,
    },
    /// "$ 1,234.50", "1.234,50 €", "(42)" -> number. `as_int` rounds.
    ParseNumber {
        #[serde(default)]
        as_int: bool,
    },
    RoundTo {
        decimals: u32,
    },
    /// Normalize unit-suffixed quantities: to = bytes | seconds | percent.
    Units {
        to: String,
    },
    /// epoch|ISO8601|RFC2822|MM/DD/YYYY -> canonical output:
    /// `rfc3339` (default) or `unix` (integer seconds).
    ParseDate {
        #[serde(default = "default_date_out")]
        output: String,
    },
    ToBool {
        #[serde(default)]
        true_values: Vec<String>,
        #[serde(default)]
        false_values: Vec<String>,
    },
    Split {
        sep: String,
        #[serde(default)]
        trim: bool,
        /// 0 = no limit.
        #[serde(default)]
        max: usize,
    },
    Join {
        sep: String,
    },
    /// Array ops (order-preserving unique).
    Unique,
    /// Lexicographic sort of an array.
    Sort,
    Take {
        count: usize,
    },
    Drop {
        count: usize,
    },
    /// Truncate by characters (not bytes) with a `…` marker when cut.
    Truncate {
        max_chars: usize,
    },
    /// Replace empty/whitespace-only text.
    ReplaceEmpty {
        with: String,
    },
    Prefix {
        with: String,
    },
    Suffix {
        with: String,
    },
}

fn default_date_out() -> String {
    "rfc3339".into()
}

fn default_enforce_off() -> String {
    "off".into()
}

impl Default for SchemaCfg {
    fn default() -> Self {
        Self {
            enforce: "off".into(),
            fields: Default::default(),
            quarantine_path: None,
        }
    }
}

/// What a PII match becomes inside a stored value.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MaskStrategy {
    /// Keep first/last chars, stars between: `jo***@ma***m`.
    #[default]
    Redact,
    /// Stable pseudonym: first 2 chars + sha256[..8] — joins across runs
    /// still work, the original does not.
    Hash,
    /// Remove entirely.
    Drop,
}

/// Record schema for a target: what fields must look like when they reach
/// the sink. `off` (default) changes nothing about today's behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SchemaCfg {
    /// off | coerce | strict — coerce repairs what it can (and counts a
    /// drift); strict quarantines the whole record on any violation.
    /// Empty string is accepted as `off` for hand-written configs.
    #[serde(default = "default_enforce_off")]
    pub enforce: String,
    /// Declared types per field name: text|number|int|bool|date|list|json.
    /// Unlisted fields pass through.
    pub fields: std::collections::BTreeMap<String, String>,
    /// JSONL of quarantined records (with reasons). Required under strict.
    pub quarantine_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum SinkCfg {
    Jsonl {
        path: PathBuf,
    },
    Csv {
        path: PathBuf,
    },
    #[serde(rename = "sqlite")]
    Sqlite {
        path: PathBuf,
    },
    /// Batched webhook push: records are posted as a JSON array to `url`.
    /// Lets a client integrate harvest with zero shared storage.
    Http {
        url: String,
        #[serde(default = "default_batch_max")]
        batch_max: usize,
        #[serde(default = "default_http_timeout")]
        timeout_secs: u64,
        /// Optional value for an `Authorization` header (e.g. "Bearer xyz").
        #[serde(default)]
        auth: Option<String>,
    },
    /// Postgres sink (build with `--features postgres`). `dsn` is libpq
    /// style: "host=… port=… user=… password=… dbname=…".
    Postgres {
        dsn: String,
    },
    /// Directory-partitioned dataset delivery: dir/dt=YYYY-MM-DD/part-*.jsonl
    /// (S3-compatible layout; `partition: "none"` disables date bucketing).
    Dataset {
        dir: PathBuf,
        #[serde(default = "default_partition")]
        partition: String,
    },
    /// Records only counted, not stored (load testing).
    Null {},
}

fn default_ckpt_secs() -> u64 {
    30
}
fn default_ckpt_pages() -> usize {
    250
}

fn default_partition() -> String {
    "day".into()
}

fn default_batch_max() -> usize {
    200
}
fn default_http_timeout() -> u64 {
    10
}

impl Default for SinkCfg {
    fn default() -> Self {
        Self::Jsonl {
            path: PathBuf::from("out/results.jsonl"),
        }
    }
}

/// The full set of keys a config document may carry at the top level.
/// Kept honest by `top_level_keys_match_serialization` below: if a field
/// is added to `Config`, the list (and thus typo detection) updates too.
impl Config {
    pub const TOP_LEVEL_KEYS: &'static [&'static str] = &[
        "admin_token",
        "alerts",
        "attest_key_path",
        "audit_path",
        "block_backoff_secs",
        "block_breaker_count",
        "browser",
        "ckpt_every_pages",
        "ckpt_every_secs",
        "ckpt_path",
        "concurrency",
        "config_version",
        "connect_timeout_secs",
        "default_policy",
        "dlq_max_attempts",
        "dlq_path",
        "domain_policies",
        "events_path",
        "max_inflight_per_proxy",
        "max_pages",
        "max_records_per_run",
        "max_response_bytes",
        "max_retries",
        "max_single_wait_ms",
        "metrics_addr",
        "name",
        "output",
        "proxies",
        "redis_url",
        "request_timeout_secs",
        "respect_robots",
        "retry_base_ms",
        "retry_deterministic",
        "revalidate_store",
        "revalidate_ttl_secs",
        "robots_ttl_secs",
        "schedule",
        "scheduling",
        "targets",
        "transport_breaker",
        "user_agent",
    ];

    pub fn load(path: &std::path::Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        // Secrets are injectable: a config string that is exactly `${NAME}`
        // (whole value, nothing else) is replaced by the environment variable
        // at load time. Undefined variables are a hard error — silently
        // shipping `${HARVEST_PG_DSN}` as a literal password would be worse.
        // The attestation hash stays over the file as written on disk, so
        // env indirection does not change the recorded policy hash.
        let expanded = Self::expand_env_refs(&raw)?;
        let value: serde_json::Value =
            serde_json::from_str(&expanded).context("config is not valid JSON")?;
        // Unknown top-level keys are a hard error (a key that never fires
        // is worse than a key that fails loudly — and the migration step
        // that rewrites documents must run on known-good input).
        crate::migrate::check_unknown_keys(&value, Self::TOP_LEVEL_KEYS)
            .map_err(anyhow::Error::msg)?;
        // Schema-version migration: v1 (absent) is accepted and stamped;
        // newer-than-this-build refuses. Warnings flow into `load_warnings`
        // for `harvest validate` / `config inspect` to surface.
        let (doc, warnings) = crate::migrate::migrate(&value).map_err(anyhow::Error::msg)?;
        let mut cfg: Config = serde_json::from_value(doc).context("config is not valid JSON")?;
        cfg.load_warnings = warnings;
        cfg.source_hash = Some(hash_bytes(raw.as_bytes()));
        cfg.source_path = Some(path.to_path_buf());
        cfg.validate()?;
        Ok(cfg)
    }

    fn expand_env_refs(raw: &str) -> Result<String> {
        fn is_ref(s: &str) -> Option<&str> {
            let inner = s.strip_prefix("${")?.strip_suffix('}')?;
            let mut bytes = inner.bytes();
            let first = bytes.next()?;
            if !(first.is_ascii_alphabetic() || first == b'_') {
                return None;
            }
            if inner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            {
                Some(inner)
            } else {
                None
            }
        }
        fn walk(v: &mut serde_json::Value, missing: &mut Vec<String>) {
            match v {
                serde_json::Value::Object(map) => {
                    for (_, val) in map.iter_mut() {
                        walk(val, missing);
                    }
                }
                serde_json::Value::Array(vals) => {
                    for val in vals.iter_mut() {
                        walk(val, missing);
                    }
                }
                serde_json::Value::String(s) => {
                    if let Some(name) = is_ref(s) {
                        match std::env::var(name) {
                            Ok(val) => *s = val,
                            Err(_) => missing.push(name.to_string()),
                        }
                    }
                }
                _ => {}
            }
        }
        let mut v: serde_json::Value =
            serde_json::from_str(raw).context("config is not valid JSON")?;
        let mut missing = Vec::new();
        walk(&mut v, &mut missing);
        if !missing.is_empty() {
            missing.sort();
            missing.dedup();
            anyhow::bail!(
                "config references undefined environment variable(s): {} — export them \
                 (or replace the `${{...}}` references with literal values)",
                missing.join(", ")
            );
        }
        Ok(v.to_string())
    }

    /// Stable sha256 of the (re)serialized config — attestation anchor for
    /// run history: "this exact policy produced these results".
    pub fn hash(&self) -> String {
        match serde_json::to_vec(self) {
            Ok(v) => hash_bytes(&v),
            Err(_) => "unknown".into(),
        }
    }

    /// Soft production review: things a config MAY legitimately do but that
    /// an operator running against third-party sites should decide about
    /// consciously. Never errors — `validate` owns hard rules; this is the
    /// second pair of eyes surfaced by `harvest validate`,
    /// `harvest status --json`, and `GET /config`.
    pub fn production_audit(&self) -> Vec<String> {
        let mut w = Vec::new();
        if !self.respect_robots {
            w.push(
                "respect_robots=false — only defensible for wholly-owned or \
                 contractually-cleared origins"
                    .into(),
            );
        }
        if self.default_policy.requests_per_sec > 5.0 {
            w.push(format!(
                "pace {:.1} req/s/host exceeds 5/s — verify the origin contract allows it",
                self.default_policy.requests_per_sec
            ));
        }
        if self.max_retries >= 8 {
            w.push(format!(
                "max_retries={} is hammering-adjacent (8..10 allowed, but most sites \
                 treat it as abuse)",
                self.max_retries
            ));
        }
        if self.concurrency > 64 {
            w.push(format!(
                "concurrency={} is aggressive for a polite crawler; per-host pacing \
                 still applies but bursts will look deliberate",
                self.concurrency
            ));
        }
        let response_reservation = self
            .max_response_bytes
            .saturating_mul(self.concurrency.clamp(1, 64) as u64);
        if response_reservation > 256 * 1024 * 1024 {
            w.push(format!(
                "max_response_bytes={} × {} effective workers reserves up to {} MiB for response bodies — reduce concurrency or the body cap if memory is tight",
                self.max_response_bytes,
                self.concurrency.clamp(1, 64),
                response_reservation / (1024 * 1024)
            ));
        }
        if self.user_agent.to_lowercase().contains("mozilla") {
            w.push(
                "UA contains \"mozilla\": validate rejects browser-spoofing, \
                    this build only warns here for completeness"
                    .into(),
            );
        }
        w
    }

    pub fn validate(&mut self) -> Result<()> {
        self.browser.validate()?;
        if self
            .browser
            .user_agent_override
            .as_ref()
            .is_some_and(|ua| ua != &self.user_agent)
        {
            bail!(
                "browser.user_agent_override must match the configured honest user_agent exactly"
            );
        }
        if self.browser.enabled && !cfg!(feature = "browser") {
            bail!(
                "browser runtime is enabled in config but this binary lacks `--features browser`"
            );
        }
        if let Some(url) = &self.redis_url {
            validate_redis_url(url)?;
        }
        if self.targets.len() > 128 {
            bail!("configuration supports at most 128 targets");
        }
        if self.targets.iter().any(|t| t.extract.len() > 1024) {
            bail!("a target supports at most 1024 extract fields");
        }
        if self.targets.is_empty() {
            bail!("no targets configured");
        }
        if self.user_agent.trim().is_empty()
            || reqwest::header::HeaderValue::from_str(&self.user_agent).is_err()
        {
            bail!("user_agent must be a non-empty, valid HTTP header with an honest bot identity");
        }
        fn http_url(label: &str, value: &str) -> Result<()> {
            if value.len() > 8192 || value.chars().any(char::is_control) {
                bail!("{label} must be at most 8192 bytes without control characters");
            }
            let u = url::Url::parse(value).with_context(|| format!("{label}: invalid URL"))?;
            if !matches!(u.scheme(), "http" | "https") || u.host_str().is_none() {
                bail!("{label} must be an absolute http(s) URL with a host");
            }
            Ok(())
        }
        fn rate_policy(label: &str, p: &DomainPolicy) -> Result<()> {
            if !p.requests_per_sec.is_finite() || p.requests_per_sec <= 0.0 {
                bail!("{label}.requests_per_sec must be finite and greater than zero");
            }
            if p.requests_per_sec < 1.0 / 3600.0 {
                bail!("{label}.requests_per_sec must be at least one request/hour; use schedule for longer intervals");
            }
            if p.burst == 0 {
                bail!("{label}.burst must be at least 1 (zero never grants a request permit)");
            }
            if p.min_interval_ms > 3_600_000 {
                bail!(
                    "{label}.min_interval_ms must be <= 3600000; use schedule for longer intervals"
                );
            }
            Ok(())
        }
        rate_policy("default_policy", &self.default_policy)?;
        for (host, policy) in &self.domain_policies {
            rate_policy(&format!("domain_policies.{host}"), policy)?;
        }
        if self.respect_robots && self.user_agent.to_lowercase().contains("mozilla") {
            bail!(
                "user_agent looks like a browser-spoofing string; use an honest bot UA \
                 (name + contact URL). Harvest is compliance-first by design."
            );
        }
        if self.concurrency == 0 {
            self.concurrency = 1;
        }
        if self.ckpt_path.is_some() {
            if self.ckpt_every_pages == 0 {
                self.ckpt_every_pages = 1;
            }
            if self.ckpt_every_secs == 0 {
                self.ckpt_every_secs = 5;
            }
        }
        // Ids are the join keys of everything downstream (metrics labels,
        // dataset partition dirs, history lines, proxy selection): validate
        // the shape and uniqueness here, not at the first collision.
        {
            let mut seen: std::collections::BTreeSet<&str> = Default::default();
            for t in &self.targets {
                crate::ids::TargetId::parse(&t.id)
                    .map_err(|e| anyhow::anyhow!("target id {:?} is not usable: {e}", t.id))?;
                if !seen.insert(t.id.as_str()) {
                    anyhow::bail!(
                        "duplicate target id `{}` — every target needs a unique id",
                        t.id
                    );
                }
            }
            // (duplication of proxy ids keeps its established message below)
            for p in &self.proxies {
                crate::ids::ProxyId::parse(&p.id)
                    .map_err(|e| anyhow::anyhow!("proxy id {:?} is not usable: {e}", p.id))?;
            }
        }
        for t in &self.targets {
            if t.max_pages == 0 {
                bail!(
                    "target {}: max_pages = 0 collects nothing; raise the target budget or remove the target",
                    t.id
                );
            }
            http_url(&format!("target {} start_url", t.id), &t.start_url)?;
            t.browser
                .validate()
                .map_err(|e| anyhow::anyhow!("target {}: {e}", t.id))?;
            if t.browser.enabled && !self.browser.enabled {
                bail!(
                    "target {} requests browser transport but browser.enabled is false",
                    t.id
                );
            }
            if t.browser.enabled && !cfg!(feature = "browser") {
                bail!("target {} requests browser transport but this binary lacks `--features browser`", t.id);
            }
            if let Some(tree) = &t.selector_tree {
                tree.validate()
                    .map_err(|e| anyhow::anyhow!("target {} selector_tree: {e}", t.id))?;
            }
            if let Some(selector) = &t.link_selector {
                scraper::Selector::parse(selector)
                    .map_err(|_| anyhow::anyhow!("target {}: invalid link_selector", t.id))?;
            }
            if t.extract.is_empty() && t.selector_tree.is_none() {
                bail!("target {}: no extract rules or selector_tree", t.id);
            }
            // Transform + schema pipeline: compiled here so `harvest
            // validate` catches bad regexes/types, not the crawl loop at
            // 03:00. Same constructor the engine uses at runtime.
            if !t.extract.is_empty() {
                crate::transform::Pipeline::compile(&t.extract)
                    .map_err(|e| anyhow::anyhow!("target {}: {e:#}", t.id))?;
            }
            match t.schema.enforce.as_str() {
                "off" | "coerce" | "strict" => {}
                other => bail!(
                    "target {}: schema.enforce must be off | coerce | strict (got `{other}`)",
                    t.id
                ),
            }
            for (field, ty) in &t.schema.fields {
                if !matches!(
                    ty.as_str(),
                    "text" | "number" | "int" | "bool" | "date" | "list" | "json"
                ) {
                    bail!(
                        "target {}: unknown schema type `{ty}` for field `{field}` \
                         (use text|number|int|bool|date|list|json)",
                        t.id
                    );
                }
            }
            if let Some(q) = &t.schema.quarantine_path {
                path_ok(&format!("target {} schema.quarantine_path", t.id), q)?;
            }
            if t.schema.enforce == "strict" && t.schema.quarantine_path.is_none() {
                bail!(
                    "target {}: schema.enforce=strict requires schema.quarantine_path                      (otherwise quarantined records would silently vanish)",
                    t.id
                );
            }
            match t.mode.as_str() {
                "html" | "json" => {}
                other => bail!("target {}: mode `{other}` invalid (use html|json)", t.id),
            }
            if t.mode == "json" && t.follow_links {
                bail!(
                    "target {}: json mode never follows links — set follow_links=false",
                    t.id
                );
            }
            if t.mode == "html" && t.link_selector.is_some() && !t.follow_links {
                // harmless: link_selector only applies when follow_links is set
            }
            if let Some(su) = &t.sitemap_url {
                http_url(&format!("target {} sitemap_url", t.id), su)?;
            }
            for (field, rule) in &t.extract {
                if t.mode == "json" {
                    if !rule.css.is_empty() || rule.attr.is_some() {
                        bail!(
                            "target {id}: field `{field}` uses css/attr, but mode=json expects `path` only",
                            id = t.id
                        );
                    }
                    let p = rule.path.as_deref().with_context(|| {
                        format!("target {}: field `{field}` needs a JSONPath (`path`)", t.id)
                    })?;
                    crate::extract::compile_path(p).map_err(|e| {
                        anyhow::anyhow!("target {}: field `{field}` bad path: {e}", t.id)
                    })?;
                } else {
                    if rule.path.is_some() {
                        bail!(
                            "target {id}: field `{field}` uses `path`, but mode=html expects `css`",
                            id = t.id
                        );
                    }
                    scraper::Selector::parse(&rule.css).map_err(|e| {
                        anyhow::anyhow!(
                            "target {id}: field `{field}` has invalid CSS selector `{css}`: {e:?}",
                            id = t.id,
                            css = rule.css
                        )
                    })?;
                }
            }
        }
        let mut ids = std::collections::HashSet::new();
        for p in &self.proxies {
            if !ids.insert(p.id.clone()) {
                bail!("duplicate proxy id `{}`", p.id);
            }
            if let Some(u) = &p.url {
                let _ = reqwest::Proxy::all(u)
                    .with_context(|| format!("proxy {}: invalid url", p.id))?;
            }
        }
        match &self.output {
            SinkCfg::Postgres { dsn } => {
                if dsn.trim().is_empty() {
                    bail!("postgres sink: dsn must not be empty");
                }
                #[cfg(feature = "postgres")]
                crate::pgsink::validate_dsn(dsn)?;
                #[cfg(not(feature = "postgres"))]
                bail!("postgres sink requested but binary lacks `--features postgres`");
            }
            SinkCfg::Http {
                url,
                timeout_secs,
                batch_max,
                auth,
            } => {
                http_url("HTTP sink url", url)?;
                if !(1..=600).contains(timeout_secs) {
                    bail!("HTTP sink timeout_secs must be 1..=600");
                }
                if !(1..=1024).contains(batch_max) {
                    bail!("HTTP sink batch_max must be 1..=1024");
                }
                if auth
                    .as_deref()
                    .is_some_and(|a| reqwest::header::HeaderValue::from_str(a).is_err())
                {
                    bail!("HTTP sink auth is not a valid HTTP header");
                }
            }
            #[cfg(not(feature = "sqlite"))]
            SinkCfg::Sqlite { .. } => {
                bail!("sqlite sink requested but binary lacks `--features sqlite`");
            }
            SinkCfg::Dataset { partition, .. } if !matches!(partition.as_str(), "day" | "none") => {
                bail!("dataset sink: partition must be \"day\" or \"none\", got \"{partition}\"");
            }
            _ => {}
        }
        // ---- resource & security limits --------------------------------------
        // Everything here either bounds a resource (memory, sockets, retry
        // storms) or closes an unauthenticated exposure. Values outside these
        // ranges are almost always typos or footguns, and we refuse to run
        // "loudly broken" rather than "quietly dangerous".
        if self.concurrency > 512 {
            bail!(
                "concurrency {} exceeds 512: workers multiply sockets, buffers and \
                 per-host timers; scale out with `harvest worker` (Redis queue) instead",
                self.concurrency
            );
        }
        if !(1..=600).contains(&self.request_timeout_secs) {
            bail!(
                "request_timeout_secs must be 1..=600 (0 would wait forever on a stalled server)"
            );
        }
        if !(1..=60).contains(&self.connect_timeout_secs) {
            bail!("connect_timeout_secs must be 1..=60");
        }
        if self.max_retries > 10 {
            bail!(
                "max_retries {} is hammering, not retrying (limit: 10) — sustained 5xx is a \
                 reason to back off, and per-403 backoff already has its own breaker",
                self.max_retries
            );
        }
        if self.retry_base_ms > 60_000 {
            bail!("retry_base_ms above 60000 belongs in politeness policy, not retry backoff");
        }
        if self.max_response_bytes < 4096 {
            bail!(
                "max_response_bytes={} is below the 4096-byte minimum; raise it to fit a real page",
                self.max_response_bytes
            );
        }
        if self.max_response_bytes > MAX_RESPONSE_BYTES_CEILING {
            bail!(
                "max_response_bytes={} exceeds the hard 64 MiB per-response ceiling",
                self.max_response_bytes
            );
        }
        let effective_workers = self.concurrency.clamp(1, 64) as u64;
        let response_reservation = self.max_response_bytes.saturating_mul(effective_workers);
        if response_reservation > MAX_IN_FLIGHT_RESPONSE_BYTES {
            bail!(
                "max_response_bytes × effective workers reserves {} MiB, above the 1024 MiB in-flight response budget; reduce the response cap or concurrency",
                response_reservation / (1024 * 1024)
            );
        }
        if self.max_pages == 0 {
            bail!("max_pages = 0 collects nothing; drop the field (default applies) or raise it");
        }
        // Engine-layer scheduling bounds. Values outside these ranges are
        // typos with operational consequences (a 64-worker ceiling is what
        // the engine clamps to anyway; >16 restarts would mask a crash-loop
        // instead of surfacing it), so validate says no at config time.
        match self.scheduling.mode.as_str() {
            "fifo" | "priority" | "fair" => {}
            other => bail!("scheduling.mode must be fifo | priority | fair (got `{other}`)"),
        }
        if self.scheduling.per_target_inflight > 64 {
            bail!("scheduling.per_target_inflight must be <= 64 (engine worker cap)");
        }
        if self.scheduling.worker_restarts > 16 {
            bail!(
                "scheduling.worker_restarts above 16 hides a crash-loop; find the panic \
                 instead (limit: 16)"
            );
        }
        if self.scheduling.retry_budget_per_min > 100_000 {
            bail!("scheduling.retry_budget_per_min above 100000 is not a budget (limit: 100000; 0 = unlimited)");
        }
        if let Some(b) = &self.transport_breaker {
            if b.min_consecutive < 2 {
                bail!(
                    "transport_breaker.min_consecutive must be >= 2: a single failure is \
                     not a pattern"
                );
            }
            if b.cooldown_secs > 3600 {
                bail!("transport_breaker.cooldown_secs must be <= 3600 (an hour is generous)");
            }
            if !(1..=16).contains(&b.recovery_successes) {
                bail!("transport_breaker.recovery_successes must be 1..=16");
            }
        }
        // Proxy pool: bounded size, transport schemes only. Credentials are
        // allowed (they are how proxies work) but must never reach logs —
        // fetcher redacts; validate rejects non-proxy schemes with a usable
        // message before any client is built.
        if self.proxies.len() > 256 {
            bail!(
                "{} proxies: pool growth is config-only by design; keep it under 256",
                self.proxies.len()
            );
        }
        for p in &self.proxies {
            if let Some(u) = &p.url {
                let parsed = url::Url::parse(u)
                    .with_context(|| format!("proxy {}: unparseable url", p.id))?;
                if !matches!(parsed.scheme(), "http" | "https" | "socks5" | "socks5h") {
                    bail!(
                        "proxy {}: scheme `{}` unsupported (use http, https, socks5 or socks5h)",
                        p.id,
                        parsed.scheme()
                    );
                }
                if parsed.host_str().unwrap_or("").is_empty() {
                    bail!("proxy {}: url has no host", p.id);
                }
            }
        }

        // Admin surface: token policy + bind-address coupling. Loopback bind
        // without a token is the documented dev mode; anything routable MUST
        // authenticate — we refuse to start an open admin API on a public
        // interface rather than trusting operator memory.
        {
            let addr = self.metrics_addr.trim();
            // An empty addr is a valid, documented choice: it disables the
            // admin server entirely (engine/runner check for exactly that).
            // We only police the case where a server will actually bind.
            let host = addr
                .rsplit_once(':')
                .map(|(h, _)| h.trim_start_matches('[').trim_end_matches(']'))
                .unwrap_or("");
            let port_ok = addr
                .rsplit_once(':')
                .and_then(|(_, p)| p.parse::<u16>().ok())
                .is_some();
            if let Some(token) = &self.admin_token {
                if token.len() < 12
                    || token.trim() != token
                    || !token.bytes().all(|b| b.is_ascii_graphic())
                    || token.len() > 512
                    || reqwest::header::HeaderValue::from_str(token).is_err()
                {
                    bail!("admin_token must be at least 12 and at most 512 characters and a valid HTTP header value");
                }
            }
            if !addr.is_empty() {
                if host.is_empty() || !port_ok {
                    bail!("metrics_addr `{addr}` must be host:PORT (port 0..=65535; 0 = random)");
                }
                let loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
                let bind_all = host.is_empty() || host == "0.0.0.0" || host == "*";
                match &self.admin_token {
                Some(t) if t.trim().len() < 12 => bail!(
                    "admin_token must be at least 12 characters of real secret (generate: \
                     `head -c24 /dev/urandom | base64`); short tokens are brute-forceable"
                ),
                None if !loopback => bail!(
                    "metrics_addr `{addr}`{} has no admin_token: /metrics, /stats and /recent \
                     would be world-readable. Bind to 127.0.0.1 for local dev or set \
                     \"admin_token\": \"${{HARVEST_ADMIN_TOKEN}}\" (injected from the environment).",
                    if bind_all { " (binds ALL interfaces!)" } else { "" }
                ),
                _ => {}
            }
            }
        }

        // Outbound sinks/queue: schemes checked before anything connects.
        if let Some(u) = self.alerts.webhook_url.as_deref().filter(|u| !u.is_empty()) {
            let parsed = url::Url::parse(u).context("alerts.webhook_url is not a URL")?;
            if !matches!(parsed.scheme(), "http" | "https") {
                bail!("alerts.webhook_url must be http(s):// — the payload is run metadata");
            }
            if parsed.scheme() == "http"
                && !matches!(
                    parsed.host_str().unwrap_or(""),
                    "127.0.0.1" | "localhost" | "::1"
                )
            {
                tracing::warn!(
                    "alerts webhook is plain HTTP to an off-box host; run metadata \
                     (record counts, target ids) crosses the wire unencrypted"
                );
            }
        }
        if !self.alerts.min_severity.is_empty()
            && !matches!(
                self.alerts.min_severity.as_str(),
                "info" | "warning" | "error"
            )
        {
            bail!(
                "alerts.min_severity must be info, warning or error (got `{}`)",
                self.alerts.min_severity
            );
        }
        if let Some(u) = self.redis_url.as_deref().filter(|u| !u.is_empty()) {
            let parsed = url::Url::parse(u).context("redis_url is not a URL")?;
            if !matches!(parsed.scheme(), "redis" | "rediss") {
                bail!("redis_url must be redis:// or rediss:// (use rediss:// for remote Redis)");
            }
            if parsed.scheme() == "redis"
                && parsed.password().is_some()
                && !matches!(
                    parsed.host_str().unwrap_or(""),
                    "127.0.0.1" | "localhost" | "::1"
                )
            {
                bail!(
                    "redis_url sends AUTH over plain TCP to a remote host — use rediss:// \
                     (TLS) or a unix socket"
                );
            }
        }
        if let SinkCfg::Postgres { dsn } = &self.output {
            let looks_like_url =
                dsn.trim().starts_with("postgres://") || dsn.trim().starts_with("postgresql://");
            if !looks_like_url && !dsn.contains("dbname=") {
                bail!("postgres sink dsn must be a postgres:// URL or key=value pairs including dbname=");
            }
        }

        // Filesystem targets: reject traversal components. The engine only
        // writes where the config tells it to; keeping `..` out of configured
        // paths means a dropped-in config cannot aim writes outside its
        // intended tree by construction, and operators see it at validate.
        fn path_ok(label: &str, p: &std::path::Path) -> Result<()> {
            if p.as_os_str().is_empty() {
                bail!("{label} is empty");
            }
            if p.components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!(
                    "{label} `{}` contains `..` — refusing paths that climb outside their base",
                    p.display()
                );
            }
            Ok(())
        }
        for (label, p) in [
            ("dlq_path", self.dlq_path.as_deref()),
            ("ckpt_path", self.ckpt_path.as_deref()),
            ("revalidate_store", self.revalidate_store.as_deref()),
            ("audit_path", self.audit_path.as_deref()),
            ("attest_key_path", self.attest_key_path.as_deref()),
            ("events_path", self.events_path.as_deref()),
            (
                "schedule.history_path",
                Some(self.schedule.history_path.as_path()),
            ),
        ] {
            if let Some(p) = p {
                path_ok(label, p)?;
            }
        }
        match &self.output {
            SinkCfg::Jsonl { path } | SinkCfg::Csv { path } | SinkCfg::Sqlite { path, .. } => {
                path_ok("output path", path)?;
            }
            SinkCfg::Dataset { dir, .. } => path_ok("output dir", dir)?,
            _ => {}
        }
        if self.schedule.every_secs == 0 && self.schedule.runs == 0 {
            bail!(
                "schedule footgun: every_secs=0 with runs=0 would loop forever back-to-back; \
                 set schedule.every_secs >= 1 or schedule.runs >= 1"
            );
        }
        Ok(())
    }

    pub fn policy_for(&self, host: &str) -> DomainPolicy {
        self.domain_policies
            .get(host)
            .or_else(|| {
                self.domain_policies
                    .iter()
                    .find(|(key, _)| {
                        key.trim_end_matches('.')
                            .eq_ignore_ascii_case(host.trim_end_matches('.'))
                    })
                    .map(|(_, policy)| policy)
            })
            .cloned()
            .unwrap_or_else(|| self.default_policy.clone())
    }
}

/// Remove credentials from a URL for logging and persisted observations:
/// `http://u:p@host:80/x?y=1` becomes `http://host:80`. Usernames are
/// credentials too, so no userinfo survives. Once a URL carries userinfo we
/// treat its entire tail as sensitive. A URL WITHOUT userinfo passes through
/// unchanged except that known-sensitive query values are masked. An
/// unparseable string is reduced to its `scheme://host` origin at best, or
/// `[redacted]`: the string itself may be the secret, so we never echo it raw.
pub fn redact_credentials(s: &str) -> String {
    if s.len() > 8192 {
        return redact_url_origin(s);
    }
    if let Ok(u) = url::Url::parse(s) {
        if u.password().is_some() || !u.username().is_empty() {
            return redact_url_origin(s);
        }
        let pairs: Vec<(String, String)> = u
            .query_pairs()
            .map(|(k, v)| {
                let private = sensitive_name(&k);
                (
                    k.into_owned(),
                    if private {
                        "***".into()
                    } else {
                        v.into_owned()
                    },
                )
            })
            .collect();
        if pairs.iter().any(|(_, v)| v == "***") {
            let mut safe = u.clone();
            safe.query_pairs_mut().clear().extend_pairs(pairs);
            safe.set_fragment(None);
            return safe.to_string();
        }
        return s.to_string();
    }
    // Unparseable: keep at most scheme + host, replace any userinfo with `***`.
    match s.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            match authority.rsplit_once('@') {
                Some((_, host)) => {
                    format!("{scheme}://***@{}", host.split(':').next().unwrap_or(host))
                }
                None => format!("{scheme}://{authority}"),
            }
        }
        None => "[redacted]".to_string(),
    }
}

/// Origin only (`scheme://host[:port]`) — for logging webhooks whose secret
/// lives in the path (Slack style) or query.
pub fn redact_url_origin(s: &str) -> String {
    match url::Url::parse(s) {
        Ok(u) => {
            let mut out = format!("{}://{}", u.scheme(), u.host_str().unwrap_or("?"));
            if let Some(p) = u.port() {
                out.push(':');
                out.push_str(&p.to_string());
            }
            out
        }
        Err(_) => "[redacted]".to_string(),
    }
}

pub(crate) fn hash_bytes(b: &[u8]) -> String {
    use sha2::Digest as _;
    let d = sha2::Sha256::digest(b);
    let mut s = String::with_capacity(64);
    for x in d.iter() {
        use std::fmt::Write as _;
        let _ = write!(s, "{x:02x}");
    }
    s
}

#[cfg(test)]
mod tests;

/// Remote Redis must use platform-verified TLS; no `#insecure` bypass.
pub(crate) fn validate_redis_url(value: &str) -> Result<()> {
    let url =
        url::Url::parse(value).map_err(|_| anyhow::anyhow!("redis_url is not a valid endpoint"))?;
    if !matches!(url.scheme(), "redis" | "rediss")
        || url.host().is_none()
        || url.fragment().is_some()
    {
        bail!(
            "redis_url must use redis:// or rediss:// with a host and without insecure/fragments"
        );
    }
    let local = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => {
            host.eq_ignore_ascii_case("localhost")
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        }
        None => false,
    };
    if !local && url.scheme() != "rediss" {
        bail!("remote Redis requires rediss:// (verified TLS; plaintext is loopback-only)");
    }
    Ok(())
}

/// Starts are explicitly approved; every other URL must stay in origin,
/// credentials, target path scope and depth, including recovered work.
pub(crate) fn approved_url(target: &TargetCfg, url: &url::Url, depth: usize) -> bool {
    let Ok(start) = url::Url::parse(&target.start_url) else {
        return false;
    };
    if url.as_str().len() > 8192
        || depth > target.max_depth
        || url.origin() != start.origin()
        || url.username() != start.username()
        || url.password() != start.password()
    {
        return false;
    }
    url == &start
        || ((target.include.is_empty() || target.include.iter().any(|s| url.as_str().contains(s)))
            && !target.exclude.iter().any(|s| url.as_str().contains(s)))
}

pub(crate) fn redact_observation_text(text: &str) -> String {
    if text.to_ascii_lowercase().starts_with("bearer ") {
        return "Bearer ***".into();
    }
    static URL_MENTIONS: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = URL_MENTIONS.get_or_init(|| {
        regex::Regex::new(r#"(?i)\b[a-z][a-z0-9+.-]*://[^\s"'<>]+"#)
            .expect("static URL mention expression is valid")
    });
    re.replace_all(text, |captures: &regex::Captures<'_>| {
        redact_credentials(captures.get(0).expect("whole URL match").as_str())
    })
    .into_owned()
}

pub(crate) fn sensitive_name(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "password"
            | "passwd"
            | "pwd"
            | "secret"
            | "credential"
            | "credentials"
            | "auth"
            | "authentication"
            | "authkey"
            | "user"
            | "username"
            | "email"
            | "oauth"
            | "token"
            | "admintoken"
            | "accesstoken"
            | "refreshtoken"
            | "authtoken"
            | "apikey"
            | "accesskey"
            | "secretaccesskey"
            | "authorization"
            | "cookie"
            | "setcookie"
            | "clientsecret"
            | "signature"
            | "sig"
            | "jwt"
            | "session"
            | "sessionid"
            | "dsn"
            | "xamzsignature"
            | "xamzcredential"
            | "xamzsecuritytoken"
    )
}
/// Observation-only. Real requests and deterministic record field values are
/// never modified by this function.
pub(crate) fn redact_observation(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if sensitive_name(key) {
                    *value = serde_json::Value::String("***".into());
                } else {
                    redact_observation(value);
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                redact_observation(value);
            }
        }
        serde_json::Value::String(text) => {
            *text = redact_observation_text(text);
        }
        _ => {}
    }
}
