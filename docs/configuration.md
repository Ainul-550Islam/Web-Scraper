# Configuration

One JSON file per config (`--config`, default `config.json`). Unknown keys
are ignored; all fields have safe defaults except `targets[]` entries'
`id`/`start_url`/`extract`. `harvest validate --config f.json` checks
everything offline (selectors, URLs, sink shapes) and prints the policy hash
without touching the network.

## Secrets: environment-variable injection

Any string value that is **exactly** `${VAR_NAME}` is replaced by the
environment variable at file load. Rules:

- whole-value only: `"css": "p[title='${X}']"` stays literal (selectors are
  not secret material and must not be mangled);
- an **unset variable is a startup error** naming it — no silent literal
  fallback, because a leaked `"${PG_DSN}"` password placeholder is worse;
- the run's attestation hash (`config_sha256`) is computed over the file as
  written on disk, so env values never leak into the recorded policy hash.

Recommended secret fields: `admin_token`, `output.Postgres.dsn`,
`redis_url`, `alerts.webhook_url`, `proxies[].url`, `targets[].auth`.

## Engine-wide fields

| Field | Type | Default | Notes / validated bounds |
|---|---|---|---|
| `name` | string | `"harvest"` | run label in history/audit/records |
| `user_agent` | string | honest bot UA | **must not contain "mozilla"** while `respect_robots` (spoof strings rejected at validate) |
| `respect_robots` | bool | `true` | turning off is for sources with written permission; pacing still applies |
| `concurrency` | int | `4` | 0 clamps to 1; **≤ 512** |
| `request_timeout_secs` | int | `20` | **1..=600** (0 = wait-forever is refused) |
| `connect_timeout_secs` | int | `10` | **1..=60** |
| `max_retries` | int | `3` | transport/5xx retries per request; **≤ 10** |
| `retry_base_ms` | int | `500` | exponential base; **≤ 60000** |
| `max_single_wait_ms` | int | `15000` | per-sleep cap inside the retry loop |
| `max_response_bytes` | int | `16777216` | hard body cap (declared + streamed); **≥ 4096** |
| `max_pages` | int | `500` | global budget; **0 refused** |
| `max_inflight_per_proxy` | int | `0` | per-egress in-flight budget: at the cap a request fails fast back into the retry lane (backpressure, not a new failure mode); 0 = unlimited |
| `max_records_per_run` | int | `0` | write-side hard cap: the run drains gracefully at the limit (checkpoints, audit, exit 0 intact); 0 = unlimited |
| `block_backoff_secs` | int | `60` | ceiling for refusal backoff |
| `block_breaker_count` | int | `5` | consecutive refusals per host before abandoning it this run |
| `robots_ttl_secs` | int | `3600` | robots cache TTL |
| `default_policy` | obj | `{requests_per_sec:1.0, burst:2, min_interval_ms:500}` | per-domain pacing floor |
| `domain_policies` | map | `{}` | same shape, keyed by host; a site's `Crawl-delay` can only **tighten** these, never relax them |
| `dlq_path` | path | off | dead-letter file (see api.md) |
| `dlq_max_attempts` | int | `3` | then "dead-for-good" |
| `ckpt_path` | path | off | checkpoint file; cadence via `ckpt_every_secs` (`30`) / `ckpt_every_pages` (`250`) |
| `revalidate_store` | path | off | conditional-GET validator store |
| `revalidate_ttl_secs` | int | `0` | `0` = entries never age out |
| `audit_path` | path | off | writes `audit.json` (+`.sig` when attesting) at run end |
| `attest_key_path` | path | off | HMAC key; missing/unreadable **fails startup**, not post-crawl |
| `events_path` | path | off | structured-event JSONL log for the run (see api.md for kinds); path is `..`-checked like every other output path |
| `retry_deterministic` | bool | `false` | forces zero jitter in retry backoff — exact, reproducible delay schedules (test/debug intent) |
| `scheduling` | obj | see below | engine-layer queue/supervision policy; every default reproduces pre-scheduler behavior |
| `transport_breaker` | obj | off | per-host transport circuit breaker (connection/timeout/5xx only — refusals stay the politeness layer's domain) |

### `scheduling` — queue & supervision policy (all opt-in)

| Field | Type | Default | Meaning |
|---|---|---|---|
| `scheduling.mode` | str | `"fifo"` | `fifo` (legacy-exact order) · `priority` (High→Normal→Low, FIFO in class) · `fair` (round-robin across targets, no target starves) |
| `scheduling.max_queue_depth` | int | `0` = unbounded | when full, discovery stops accepting URLs (`queue_rejected` counts them); queued work is never evicted |
| `scheduling.per_target_inflight` | int | `0` = unbounded | `fair` mode only: max jobs executing per target; **≤ 64** |
| `scheduling.worker_restarts` | int | `0` | restarts granted to a panicking worker task before the run fails (legacy = fail on first death); **≤ 16** — more only hides a crash-loop |
| `scheduling.job_max_age_secs` | int | `0` = no deadline | a queued job older than this expires at pop time (`job_expired` event, never fetched late) |
| `scheduling.retry_budget_per_min` | int | `0` = unlimited | run-wide token bucket for retry sleeps; under a storm the engine fails fast instead of stacking waits; **≤ 100000** |

### `transport_breaker` — per-host transport circuit

| Field | Type | Default | Meaning |
|---|---|---|---|
| `min_consecutive` | int | `3` | transport/5xx failures in a row before opening; **≥ 2** (one failure is not a pattern) |
| `cooldown_secs` | int | `60` | how long an open circuit skips the host; **≤ 3600** |
| `recovery_successes` | int | `1` | half-open probe successes required to close; **1..=16** |

While a circuit is open, jobs for that host are retired to the DLQ path
with reason `breaker:open` — no sockets are opened, no requests are
hammered, and the skip is visible (`breaker_rejected` counter,
`breaker_opened/closed` events, `GET /runtime`). This never trips on
401/403/429/451: refusals are answers, not outages.
| `redis_url` | url | off | **redis:// or rediss:// only**; AUTH to a remote host over plain TCP is refused |
| `metrics_addr` | host:port | `127.0.0.1:9678` | `""` disables admin; non-loopback **requires** `admin_token`; port 0..=65535 |
| `admin_token` | string | none | with token ⇒ all admin routes authed except `/health`; **≥ 12 chars**; `""` refused; inject via `${HARVEST_ADMIN_TOKEN}` |
| `schedule.every_secs` | int | `3600` | `serve` spacing (`0` back-to-back only valid with `runs ≥ 1`) |
| `schedule.runs` | int | `0` | `0` = until stopped |
| `schedule.history_path` | path | `out/runs.jsonl` | append-only run history |
| `proxies[]` | list | `[]` | `{id, url, region}`; **≤ 256 entries**; url scheme ∈ http/https/socks5/socks5h with a host; credentials allowed in `url` but never logged |

## `targets[]` — one collection job

| Field | Default | Notes |
|---|---|---|
| `id` | — | unique label; keys per-target stats/sinks |
| `start_url` | — | parsed URL; seeds the crawl (also for `mode:"json"`) |
| `mode` | `"html"` | `html` (CSS extract) or `json` (JSONPath-lite); `json` + `follow_links:true` is a **startup error**, not a silent ignore |
| `max_depth` | `0` | link-follow generations |
| `follow_links` | `false` | discovery only when on |
| `link_selector` | none | CSS selector narrowing href discovery (html, follow_links) |
| `include` / `exclude` | `[]` | **substring** match on the absolute URL; exclude wins |
| `max_pages` | `20` | per-target budget |
| `extract` | — | map of output-field → rule; empty is an error |
| `schema` | `off` | declared record types + enforcement (`off`/`coerce`/`strict`) with quarantine lane — see below |
| `sitemap` | `false` | seed additionally from `Sitemap:` in robots.txt / `/sitemap.xml` |
| `sitemap_url` | none | explicit sitemap/index URL (validated) |

### extract rules (`FieldCfg`)

html mode: `{"css": "h1", "attr": "href"?, "all": true?}`;
json mode: `{"path": "$.items[*].title"}` — `$.a.b`, `[*]` array splat,
`[N]` index; mixed css+json usage is a named startup error. `all:true`
collects a JSON array of every match.

### `transforms` — the value pipeline (per field)

Every rule accepts `"transforms": [ … ]` — ordered ops applied after
extraction, before the record is written. **Empty = pass-through: byte-identical
to pre-pipeline output, and the engine skips the whole pass at zero cost.**
Regexes compile once per run (and once inside `harvest validate`, so a bad
pattern is a config error, never a 03:00 crawl-loop panic). Chains are
ATOMIC per field: a failing step leaves the raw value in place and reports
the reason — no half-transformed fields ever ship.

| op | options | notes |
|---|---|---|
| `trim` · `lowercase` · `uppercase` · `collapse_whitespace` | — | string steps map element-wise over `all:true` arrays |
| `regex_replace` | `pattern`, `replacement` | Rust regex; `$1`-style groups |
| `regex_extract` | `pattern`, `group` (default 0) | no match = failure (raw value kept) |
| `strip_tags` | — | removes HTML tags (quote-aware), decodes the 5 named entities + numeric refs, UTF-8 safe |
| `mask_pii` | `strategy`: `redact`\|`hash`\|`drop`; `kinds`: subset of `email`\|`phone`\|`card`\|`ipv4` | collection-time REDACTION of what you store; cards gated by Luhn, IPv4 by octet range — near-miss digit noise survives untouched; `hash` = first 2 chars + sha256[..8] (stable pseudonym, joins still work) |
| `parse_number` | `as_int` | `"$ 1,234.50"`→1234.5, `"1.234,50 €"`→1234.5, `"(42)"`→-42; a lone separator with exactly 3 trailing digits is a THOUSANDS mark (`"1.005"`→1005) — pass JSON numbers for exact fractions (numeric inputs never round-trip through text) |
| `round_to` | `decimals` | round-half-to-even on the exact binary value |
| `units` | `to`: `bytes`\|`seconds`\|`percent` | KB=1000 vs KiB=1024 distinct; bare numbers are identity for bytes/seconds, an error for percent (ambiguous) |
| `parse_date` | `output`: `rfc3339`\|`unix` | accepts epoch (s/ms), RFC3339 with offset, `YYYY-MM-DD`, IMF `Wed, 21 Oct 2015 07:28:00 GMT`, `MM/DD/YYYY`; calendar-validates (no Feb 30, leap years per Gregorian rules) |
| `to_bool` | `true_values`, `false_values` | defaults include yes/no/on/off/available/in stock; overlapping sets = config error |
| `split` | `sep`, `trim`, `max` | text→array |
| `join` | `sep` | array→text |
| `unique` · `sort` · `take` · `drop` | `count` for take/drop | array-only; non-arrays are a reported failure, not a coercion |
| `truncate` | `max_chars` | counts CHARACTERS, appends `…` when cut |
| `replace_empty` · `prefix` · `suffix` | `with` | whitespace-only counts as empty for `replace_empty` |

### `schema` — declared record contract & quarantine

```json
"schema": { "enforce": "strict",
            "fields": { "price": "number", "seen": "date", "tags": "list" },
            "quarantine_path": "out/quarantine.jsonl" }
```

| key | default | meaning |
|---|---|---|
| `enforce` | `off` | `off`: type-drift observation only (values untouched). `coerce`: repair what is mechanically repairable (text→number/bool, date normalization, comma-text→list) — each repair counts `harvest_schema_drifts`. `strict`: any violation appends the WHOLE record to `quarantine_path` (`{"url","depth","ts","reasons":["field: step: why"],"fields":{…}}`) and skips the sink write |
| `fields` | `{}` | declared types `text`\|`number`\|`int`\|`bool`\|`date`\|`list`\|`json`; unlisted fields pass; an ABSENT field is the extractor's contract, never a violation |
| `quarantine_path` | none | required under `strict` (a startup error otherwise — silently vanishing records are worse than noisy ones); goes through the same `..`-traversal check as every configured path |

Quarantining is a **data outcome, not a fetch failure**: the job still
succeeds, links found on the page are still crawled, and `records` still
counts the extraction — `quarantined` in the summary is the lane that
emptied the sink. Transform failures without a declared schema never
quarantine (no contract was promised); they count as drift and the raw
value ships.

`harvest schema --config c.json [--target id]` prints the derived draft-07
JSON Schema per target (declared types when present, otherwise inferred
from transform chains; extraction metadata under `harvest:source` /
`harvest:transforms`). Pipe it through `jq` or a schema-diff tool to gate
record-format changes in CI. Live demo config: `examples/config.pipeline.json`.

## `output` — sink selection (`kind`)

| kind | shape | notes |
|---|---|---|
| `jsonl` | `{"kind":"jsonl","path":"out/r.jsonl"}` | append; one record/line |
| `csv` | `{"kind":"csv","path":"out/r.csv"}` | header written once |
| `sqlite` | `{"kind":"sqlite","path":"out/r.db"}` | needs `--features sqlite` |
| `postgres` | `{"kind":"postgres","dsn":"host=… dbname=…"}` | needs `--features postgres`; dsn must carry `dbname=` (or be a `postgres://` URL); table `harvest_records` auto-created; re-runs are idempotent via `ON CONFLICT DO NOTHING` |
| `dataset` | `{"kind":"dataset","dir":"out/data","partition":"day"}` | `dir/dt=YYYY-MM-DD/part-*.jsonl`; `partition:"none"` disables bucketing |
| `http` | `{"kind":"http","url":"https://…","batch_max":200,"timeout_secs":10,"auth":"${SINK_TOKEN}"?}` | batched JSON-array POST |
| `null` | `{"kind":"null"}` | counts only (load tests) |

`path`/`dir` for file sinks go through the same `..`-traversal rejection as
every other configured path.

## `alerts` — SLO webhooks

```json
"alerts": { "webhook_url": "${ALERT_WEBHOOK}", "events": ["no_data","high_block_rate","run_error"],
            "min_records": 1, "max_block_rate": 0.05,
            "per_target": { "shop": { "min_records": 10, "max_block_rate": 0.5 } } }
```

`webhook_url` must be http(s) (plain-HTTP remote ⇒ one warning line); fires
at most one POST per run; logs show the **origin only**. `no_data` counts
`written + revalidated` so a healthy all-304 re-crawl stays silent.

## Validation failures (all fatal at `validate`/startup)

no targets · browser-spoof UA with robots on · concurrency > 512 ·
timeouts out of range · retries > 10 · `retry_base_ms` > 60000 ·
`max_response_bytes` < 4096 · `max_pages` = 0 · duplicate proxy ids ·
duplicate target ids · malformed target/proxy ids (ids are slugs:
`a-z 0-9 . _ -`, ≤ 64 bytes, no leading dot, no `..`, no trailing dot — they
become path components and metric labels) ·
proxy count > 256 / bad scheme / no host · routable admin bind without
token · short/empty token · bad `metrics_addr` shape · `redis_url`
wrong scheme or remote AUTH over plaintext · non-http(s) webhook ·
postgres dsn without dbname (when the feature is missing the error names
that instead) · `..` in any configured path · empty path · invalid CSS
selector · invalid JSONPath · json-mode with `follow_links` · json field
using css or html field using `path` · missing `path` for a json field ·
`schedule.every_secs:0` with `runs:0` · sitemap_url unparsable ·
target start_url unparsable.

## Schema version, migration, and comments

Configs carry `config_version` (current: 2). The loader pipeline is strict in
the useful ways:

* **Absent ⇒ v1.** Unversioned files are treated as the v1 baseline; loading
  stamps them to the current version and records a note
  (`config_version: stamped 2 …`) in `Config::load_warnings`. The v1→v2
  migration is honestly stamp-only — no field changed between them — and the
  migration table documents that pattern for v3+.
* **Newer than the binary ⇒ refusal.** A future config is never silently
  downgraded; upgrade `harvest` instead.
* **Unknown top-level keys ⇒ refusal**, with a did-you-mean when the key is
  one edit away from a real one. A renamed field failing loudly beats a
  renamed field silently ignored.
* **`_`-prefixed keys are comments.** `_note` and friends are ignored
  everywhere (the shipped examples use them).

`harvest config migrate --config prod.json [--out new.json] [--dry-run]
[--json]` applies the migration table to the file *as written*: `${ENV}`
references stay unresolved, so migrating never copies secrets to disk.
`harvest validate` prints the migration notes plus the production audit as
`warning:` lines — warnings never move the exit code; hard rules only.

The production audit (soft, all five findings are advisory): `respect_robots`
off, any domain policy above 5 req/s, `max_retries` ≥ 8, concurrency > 64,
`max_response_bytes` > 64 MiB. `GET /config` and `harvest config inspect`
surface them as `_production`, load notes as `_warnings`, and `_backends`
(which optional storages this binary was compiled with).

## Environment variables read by the code

| Variable | Read by | Purpose | Example | Required? | Safe to expose? |
|---|---|---|---|---|---|
| `RUST_LOG` | tracing subscriber (all commands) | log filter | `info,harvest=debug` | no (default `info,harvest=debug`) | yes |
| any `${NAME}` you reference | `Config::load` | secret injection | `HARVEST_ADMIN_TOKEN=a1b2…` | required iff referenced | **no** — secret material |
| `HARVEST_TEST_REDIS` | test suite only | enables live-Redis tests | `1` | no | yes |
| `HARVEST_TEST_REDIS_URL` | test suite only | overrides redis endpoint under test | `redis://127.0.0.1:6399/9` | no | yes |
| `HARVEST_TEST_PG` | test suite only | enables live-Postgres test | `1` | no | yes |
| `HARVEST_PG_DSN` | test suite only | DSN for the live-Postgres test | `host=127.0.0.1 …` | no | **no** in CI if it carries a real password (it must not) |
| `HARVEST_ADMIN_TOKEN`, `GRAFANA_ADMIN_USER`, `GRAFANA_ADMIN_PASSWORD` | `ops/docker-compose.yml` | compose injects these into container envs, where the config's `${…}` picks them up (Grafana via its own env) | `head -c24 /dev/urandom \| base64` | for any non-loopback exposure | **no** |

## Examples

Working configs in `examples/`: `config.basic.json` (local demo),
`config.production.json` (hardened single-node: env-injected secret, audit +
attest + checkpoint + revalidation), `config.distributed.json` (redis
queue). Validate any of them offline:
`cargo run --release -- validate --config examples/config.production.json`.
