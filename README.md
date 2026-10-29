# harvest — a compliance-first Rust data engine

Fast, bounded, observable web data collection for sources you are allowed to
read: public APIs you've authenticated against, open-data portals, partner
feeds, licensed catalogs. Built on `tokio` + `rustls` for throughput, with the
courtesy machinery (rate policy, robots.txt, refusal handling) enforced *in
the engine* rather than in operator discipline.

## Contents

| README section | Full documentation |
|---|---|
| What this is / isn't · Quick start · Deployment | [docs/](docs/) index below |
| Configuration (JSON) | [docs/configuration.md](docs/configuration.md) — every field, default, env var, startup refusal |
| Distributed / sinks / exports / attestation | [docs/api.md](docs/api.md) — admin API + every file format |
| Control plane (authenticated execution) | [docs/api/24-control-plane-execution.md](docs/api/24-control-plane-execution.md) · [docs/api/25-artifact-downloads.md](docs/api/25-artifact-downloads.md) |
| Architecture | [docs/architecture.md](docs/architecture.md) — modules, lifecycle, data flow |
| Deployment (Docker/compose/prod) | [docs/deployment.md](docs/deployment.md) — topologies, DBs, health, backups |
| When something goes wrong | [docs/troubleshooting.md](docs/troubleshooting.md) — symptom → cause → check → fix |
| Hacking on harvest | [docs/development.md](docs/development.md) — commands, tests, CI, workflow |
| Security posture | [SECURITY.md](SECURITY.md) + production checklist below |
| Version history | [CHANGELOG.md](CHANGELOG.md) |
| Web console (React/TS) | [docs/web-client.md](docs/web-client.md) — pages↔routes map, security model, dev proxy, build & tests |
| Evaluating as a buyer | [docs/buyer-quickstart.md](docs/buyer-quickstart.md) (5 commands) → [docs/buyer-demo.md](docs/buyer-demo.md) (A–AC script) |
| What is verified vs claimed | [docs/release-status.md](docs/release-status.md) · [docs/capability-matrix.md](docs/capability-matrix.md) · [docs/codebase-map.md](docs/codebase-map.md) |


## What this is / what it is deliberately not

| In scope | Out of scope (by design) |
|---|---|
| Async bounded crawl/fetch with retries + jittered backoff | TLS/JA3/H2 fingerprint spoofing (curl-impersonate style) |
| robots.txt enforcement (RFC-9309-ish, TTL-cached, `Crawl-delay` honored as pacing floor) | Cloudflare/Akamai/anti-bot *bypass* of any kind |
| Per-domain token bucket + min-interval pacing | "Ban evasion" proxy rotation (hiding behind new IPs after a refusal) |
| Proxy pool with health scoring for **reliability & geo** | Anything that turns `403` into `200` by disguise |
| CSS selectors (HTML) **and** JSONPath-lite (APIs), fingerprint dedup | |
| Sitemap-driven seeding: `<urlset>` + `<sitemapindex>`, filtered through the same robots/pace rules | |
| JSONL / CSV / SQLite / **Postgres** / dataset-dir / webhook sinks | |
| Scheduler + audit history + **SLO alert webhooks** (global *and* per-target) | |
| Dead-letter queue with bounded re-attempts; **Redis distributed queue** for multi-process shards | Owning the customer's cloud credentials (S3 push = rclone on the dataset dir) |
| Prometheus metrics, status dashboard, bearer-token admin auth (fail-closed startup guards) | Response-body/redirect/retry hard caps; `${ENV}` secret injection; `..`-path rejection |
| **Checkpoint/resume** — day-long crawls survive `kill -9`, deploys and reboots | |
| **Incremental revalidation** — conditional GET (ETag/IMS) + replayed link graph | |
| **`harvest export`** — delivery folder: CSV + canonical JSONL + sha256 manifest; record-boundary part chunking, crash-resume, and a `harvest verify` gate | Parquet/arrow in-process (heavy dep; duckdb reads our JSONL natively) |
| **Politeness audit** — per-host request ledger (`audit.json`), reconciles 1:1 with the target's access log | |
| **HMAC attestation** — detached `.sig` over audit/manifest; tamper-evidence without PKI | |
| **Resource budgets** — per-egress in-flight caps and per-run record caps; at-cap is backpressure through the existing retry lane, and a capped run still exits 0 | |

When a server refuses us (`401/403/429/451`), `harvest` treats that as an
**answer**: it honors `Retry-After`, backs off exponentially, and trips a
per-domain circuit breaker. This is not a moral restriction only — it is also
what keeps a data vendor's clients off a platform's abuse queue and out of
`CFAA`/`CMA`-style disputes, which is the actual reason enterprises pay for
managed data.

## Quick start

```bash
# 1. validate a config (no network) — prints policy sha256
cargo run --release -- validate --config config.example.json
# 1b. see the record contract that config implies (draft-07 JSON Schema)
cargo run --release -- schema --config config.example.json

# 2. one-shot collection against the bundled mock source
python3 demo/server.py 8099 &
cargo run --release -- run --config demo/localhost.json
cat out/demo.jsonl

# 3. scheduler mode (demo config repeats every 10s, forever): ctrl-c stops
#    after the current run; the dashboard + history stay up between runs
cargo run --release -- serve --config demo/localhost.json

# 4. inspect the audit trail (same config's schedule.history_path)
cargo run --release -- history --config demo/localhost.json --n 20

# 5. hand the client a verifiable delivery folder
cargo run --release -- export --from out/demo.jsonl --to deliver/ \
    --history out/demo-runs.jsonl --split-target
#    -> records.csv + records.jsonl + manifest.json (per-file sha256,
#       counts, and the run's policy hash). rclone the folder anywhere.
#    big delivery? --max-part-bytes 268435456 chunks at record boundaries;
#    after a crash, re-run with --resume (validated parts are kept, never
#    trusted blindly), and gate the handoff with:
#    cargo run --release -- verify --dir deliver/   # exit 0 only if complete

# 6. prove the politeness contract — your own copy needs "audit_path" in
#    the config; the repo ships a real one from a live smoke run:
cargo run --release -- audit --file examples/output/audit.sample.json --md
#    -> per-host table: requests by status, pacing sleeps, backoffs,
#       robots denials — totals match the site's own access log exactly.
#    with "attest_key_path" set, every receipt also gets a detached HMAC
#    (verify: cargo run -- attest --verify examples/output/audit.sample.json
#              --key <the key that signed it> — fails closed otherwise).

# 7. live status while `serve` runs (another terminal), or while a `run` is
#    in flight: dashboard (zero-asset single page), metrics, stats — no
#    token needed at this loopback default; see SECURITY.md for exposure.
curl -s http://127.0.0.1:9678/health
curl -s http://127.0.0.1:9678/metrics | head
curl -s http://127.0.0.1:9678/recent | head -c 400
```

## Deployment (Docker + console + Prometheus + Grafana)

`ops/docker-compose.yml` runs the hardened stack: non-root engine (uid
10001), read-only root filesystems, dropped capabilities, health checks, and
host ports bound to `127.0.0.1` by default. Start from
`ops/harvest/config.example.json` for Compose: it binds the admin listener to
`0.0.0.0:9678` **inside the Compose network**, requires the injected
`${HARVEST_ADMIN_TOKEN}`, and stores writable state under `/var/lib/harvest`.
The published host ports remain loopback-only; do not remove the `127.0.0.1:`
prefixes without designing an authenticated TLS front end. Secrets are
provided through the host environment and are not baked into either image.
Grafana has no hard-coded password: `GRAFANA_ADMIN_PASSWORD` is mandatory.
See **SECURITY.md** for the full production checklist and startup-time
rejections (open admin binds, traversal paths, unbounded retry/timeout/size
settings).

The engine drains on both **SIGINT** and **SIGTERM**; Compose sends SIGTERM
and allows 45 seconds. `/health` and `/live` are the only unauthenticated
admin probes; other admin routes require the bearer token when configured.
Full guide: [docs/deployment.md](docs/deployment.md).

```bash
cp -n ops/harvest/config.example.json ops/harvest/config.json
# Edit the demo target and user-agent contact before starting the stack.
export HARVEST_ADMIN_TOKEN="$(head -c24 /dev/urandom | base64)"
export GRAFANA_ADMIN_PASSWORD="$(head -c18 /dev/urandom | base64)"
docker compose -f ops/docker-compose.yml up --build -d
# Console: http://127.0.0.1:8088
# Grafana: http://127.0.0.1:3000 (user admin, password is the value you generated)
```

## Configuration (JSON)

The complete field reference — every default, every validated range, the
full `${VAR}` injection contract and the startup-refusal catalog — is
[docs/configuration.md](docs/configuration.md). The section below is the
tour, kept accurate to `src/config.rs`.

```jsonc
{
  "user_agent": "harvest-demo/0.1 (contact: you@acme.example)",  // honest bot ID — validated, browser-spoof strings are rejected
  "respect_robots": true,                 // refusal-resistant default
  "concurrency": 4,
  "default_policy": { "requests_per_sec": 1.0, "burst": 2, "min_interval_ms": 500 },
  "domain_policies": { "api.partner.example": { "requests_per_sec": 5.0, "burst": 10, "min_interval_ms": 100 } },
  "block_backoff_secs": 60,               // after 401/403/429/451
  "block_breaker_count": 5,               // then abandon the domain for this run
  "proxies": [                            // for egress quality + geography
    { "id": "direct" },
    { "id": "fra-1", "url": "http://user:pass@gw.example:8000", "region": "eu-west" }
  ],
  "targets": [
    {
      "id": "catalog",
      "start_url": "https://data.partner.example/index",
      "follow_links": true, "max_depth": 2,
      "sitemap": true,                    // seed from /sitemap.xml (+ index children) first
      "mode": "html",                     // or "json" with "path": "$.x[*].y" extract rules
      "include": ["/product/"], "exclude": ["/cart"],
      "extract": {
        "title": { "css": "h1" },
        "price": { "css": "p.price" },
        "link":  { "css": "a.canonical", "attr": "href" },
        "tags":  { "css": "li.tag", "all": true }
      }
    }
  ],
  "output": { "kind": "jsonl", "path": "out/results.jsonl" },
  "dlq_path": "out/dlq.jsonl",              // dead-letter file; `run --retry-dlq` replays it
  "ckpt_path": "out/state.ckpt",            // resumable crawls: `run --resume`
  "ckpt_every_secs": 30, "ckpt_every_pages": 250, // snapshot cadence (either trigger)
  "revalidate_store": "out/validators.jsonl", // conditional GET across runs
  "revalidate_ttl_secs": 0,                 // 0 = entries never age out
  "redis_url": "redis://127.0.0.1:6379/0",  // remote fleet: rediss:// (TLS) is enforced for remote AUTH
  "max_response_bytes": 16777216,           // hard per-response body cap (default 16 MiB)
  "admin_token": "${HARVEST_ADMIN_TOKEN}",  // whole-value ${VAR} = environment injection;
                                            // omit the key entirely for localhost-only dev
  "metrics_addr": "127.0.0.1:9678"          // "" disables the admin server; binding a
                                            // routable address WITHOUT a token is refused
}
```

`output.kind`: `jsonl` | `csv` | `sqlite` (build with `--features sqlite`) |
`postgres` (build with `--features postgres`) | `dataset` | `http` (batched
webhook POST — see below) | `null` (load test). Sink records carry `run`,
`target`, `url`, `depth`, `ts`, `fingerprint` (normalized-text SHA-256),
`egress` (proxy used), and the extracted `fields`.

**Postgres sink** — `"output": { "kind": "postgres", "dsn":
"host=… user=… password=… dbname=…" }`. Writes to `harvest_records`
(auto-created, `payload JSONB`, unique index on
`(run, target, fingerprint)` + `ON CONFLICT DO NOTHING` ⇒ re-runs are
idempotent). Verified against a real PostgreSQL 17 cluster in
`tests/pg.rs` (opt-in: `HARVEST_TEST_PG=1`, skips cleanly without a DB).

**Dataset sink** — `"output": { "kind": "dataset", "dir": "out/data" }`
creates S3-friendly partitions `dir/dt=YYYY-MM-DD/part-<run>-<seq>.jsonl`;
sync with any tool you like, no cloud SDK in the engine binary.

### Sources & formats

**Sitemap seeding** — `"sitemap": true` per target (optionally `sitemap_url`):
before workers start, harvest fetches `<origin>/sitemap.xml` *through the same
politeness + robots gate* as crawl traffic, follows one level of
`<sitemapindex>` children, applies `include`/`exclude`, drops off-site and
robots-disallowed locs, and merges the rest with the start-URL crawl (visited
dedup means a page advertised twice is fetched once). A denied or broken
sitemap degrades to plain crawling with a warning — never a failed run.

**JSON / API targets** — `"mode": "json"` with `path` rules instead of `css`:

```jsonc
{ "id": "prices", "mode": "json", "follow_links": false,
  "start_url": "https://api.partner.example/v2/items",
  "extract": {
    "names": { "path": "$.items[*].name", "all": true },
    "price": { "path": "$.items[0].price_cents" }   // numbers stay numbers
  } }
```

Supported path syntax: `$`, `.key`, `[n]`, `[*]` (compiles at `validate`
time — typos fail the config, not the pipeline). Missing paths yield `null`.
Mixed-mode configs (`css` under json, `path` under html) are rejected with an
explicit error.

### Record pipeline: transforms + schema (opt-in, default-off)

Each extract rule can carry `"transforms": [...]` — ordered value ops
(`trim`, `strip_tags`, `mask_pii`, `parse_number`, `parse_date`,
`split`/`unique`, `truncate`, …23 in total) that run between extraction and
the sink; and each target can declare a `"schema"` (`off`/`coerce`/`strict`)
that repairs drifting types or quarantines contract-violating records to
their own JSONL instead of writing them. Empty transforms + `off` schema is
exactly the legacy behavior — the pipeline pass costs one branch. A
quarantined record never fails the job (links still get crawled) and never
hits the sink; `harvest_records_quarantined` and `harvest_schema_drifts`
make the lane observable, and `harvest schema --config c.json` prints the
derived draft-07 contract for CI review. Full op table:
[docs/configuration.md](docs/configuration.md). Live example:
`examples/config.pipeline.json`. Sitemap + JSON compose: an API endpoint can itself be a
sitemap consumer — but *distributed* (`harvest worker`) shards are HTML-mode
only for now; json-mode targets there fail fast at startup, not mid-run.

### HTTP webhook sink

Pushes records straight into the client's ingest endpoint — zero shared
storage between harvest and the consumer:

```jsonc
"output": {
  "kind": "http",
  "url": "https://ingest.customer.example/harvest",
  "batch_max": 200,            // records per POST (JSON array body)
  "timeout_secs": 10,
  "auth": "Bearer <token>"     // optional Authorization header
}
```

Delivery is batched + one retry per batch; failures surface at run flush, so
a broken webhook fails the run loudly instead of silently dropping data.

### SLO alert webhooks

```jsonc
"alerts": {
  "webhook_url": "https://hooks.customer.example/harvest-slo",
  "events": ["run_error", "no_data", "high_block_rate"],
  "min_records": 50,        // no_data fires when written < this
  "max_block_rate": 0.4     // high_block_rate at blocked/requests >= this
}
```

```jsonc
"alerts": { ...,
  "per_target": {                     // SLA per contract line, not just per run
    "client-a-catalog": { "min_records": 500, "max_block_rate": 0.1 },
    "client-b-prices":  { "min_records": 10 }
  }
}
```

Per-target thresholds are evaluated **first** and are what a per-customer SLO
actually means: a run can clear the global floor yet still starve one target.
Reason codes carry the target (`no_data:target=client-b-prices:written4`,
`high_block_rate:target=client-a-catalog`), so one webhook serves many SLAs.

Each fired alert POSTs `{run, ts, status, reasons, summary, policy_sha256}` —
the same policy hash from run history, so an alert is self-attesting.
Unreachable webhooks are logged, never fatal; sink failures (dead DB) are the
opposite — runs fail loudly.

### Admin auth & fail-closed startup

Set `"admin_token"` and every admin route requires
`Authorization: Bearer <token>` — timing-safe compare, constant 401 bodies
that never echo the path or any token material. The two liveness probes
`/health` (`{"status":"ok","running":bool}`) and `/live` (fixed `{"live":
true}`) are the exemptions: constant content, no config, no counters —
exactly what container/K8s probes need and safe on an otherwise token-gated
port. Unset token = open endpoints **and that is only allowed on
loopback**: `metrics_addr` bound to a routable address (or `0.0.0.0`) with
no token fails validation at startup — as do empty/short (<12 char) tokens,
malformed `redis://`/`http(s)://` endpoints, `..` in any configured path,
`max_retries > 10`, zero timeouts, `max_response_bytes` outside
4096..=64 MiB or a response-cap × effective-worker reservation above 1 GiB,
and proxy URLs with non-proxy schemes. All responses carry
`x-content-type-options: nosniff` + `cache-control: no-store`.

**Secret injection:** any config string that is *exactly* `${VAR_NAME}` is
expanded from the environment when the file is loaded; an unset variable is
a hard startup error naming it (never a literal fallback). Inline `${...}`
substrings (CSS selectors, UA text) are left untouched. This is the
mechanism `ops/docker-compose.yml` relies on — no secrets in images,
volumes, or git.

### Control plane (bounded execution)

The bounded control plane adds six command/job routes (three submissions,
two job reads, and cancellation), plus three artifact metadata/download/verify
routes:

```
POST /control/crawl            # one bounded crawl over the config's targets
POST /control/export           # render the configured run output as a delivery
POST /control/ai               # one AI-assistance round (local provider; off by default)
GET  /control/jobs             # paged job list: state, counters, artifacts, caps
GET  /control/jobs/:id         # one job: limits applied, events, reason, audit ref
POST /control/jobs/:id/cancel  # cancel a queued/running job (terminal jobs: 409)
GET  /control/artifacts/:id    # artifact metadata — never bytes, never a path
GET  /artifacts/:id            # the bytes (verification-gated, size-capped)
GET  /artifacts/:id/verify     # re-run the manifest + checksum comparison
```

When `admin_token` is configured, these routes require its bearer token
(`/health` and `/live` stay open). Without a token, the normal loopback-only
policy applies. A command is validated against fixed ceilings, clamped down to the attached
configuration, and run as one job with a deadline; at most 8 run at once and
64 may be in flight — beyond that the answer is a clean 503, never a queue
that grows without bound. Crawl/export/AI submissions accept an
`Idempotency-Key`, so a retry returns the original job instead of starting a
second one; cancel is state-aware and a terminal-job repeat returns 409.

The crawl runs on the same engine the scheduler uses, the export on the same
exporter as `harvest export`, the AI round on the same client as the pipeline.
There is no second executor, queue, retry engine, auth system or export
implementation, and no fake progress: a job reports `queued`, `running`,
`completed`, `failed` or `cancelled` — exactly what the server decided.

Details: [`docs/api/24-control-plane-execution.md`](docs/api/24-control-plane-execution.md)
and [`docs/api/25-artifact-downloads.md`](docs/api/25-artifact-downloads.md).

### Managed scrape API (tenant-facing, off by default)

An optional second surface on the same port, for customers who call the engine
over HTTP instead of driving the console. It is **off unless
`managed_api.enabled` is true**, and every route needs a tenant API key — a
key is stored as a SHA-256 digest, carries a closed set of scopes and per-key
limits, and its plaintext is shown exactly once at creation or rotation.

```
POST /v1/scrape                 # one URL -> text/html/json/markdown/screenshot
POST /v1/jobs                   # queue a bounded job (202; repeats on a schedule)
GET  /v1/jobs                   # the tenant's pending jobs
GET  /v1/jobs/:id               # one job, with its finished scrape
POST /v1/jobs/:id/cancel        # cancel a live job (terminal jobs: 409)
GET  /v1/usage                  # metering from the append-only ledger
GET  /v1/openapi.json           # the published contract (3.1, generated)
POST /v1/admin/tenants          # operator: create a tenant      (console bearer)
GET  /v1/admin/tenants          # operator: list tenants
POST /v1/admin/keys             # operator: issue a key
GET  /v1/admin/keys             # operator: list a tenant's keys
POST /v1/admin/keys/:id/rotate|revoke|suspend
GET  /v1/admin/usage            # operator: the same report, unfiltered
POST /v1/admin/usage/export     # operator: JSONL or CSV download
```

A key's limits are enforced, not advisory: a token bucket for `rps`/`burst`, a
per-key concurrency cap, a monthly quota, a hard response-byte cap and an
optional domain allowlist. Exceeding any of them is a `429` or `403` with the
reason, before any work happens. `POST /v1/scrape` honours an
`idempotency-key`, so a retry returns the stored response instead of being
charged twice.

Compliance is not a console feature here: robots.txt, per-domain pacing,
circuit breakers and the SSRF guard apply to every customer request exactly as
they do to the scheduler's. Every URL is resolved and checked before the fetch,
and re-checked on every redirect hop; loopback, private, link-local, multicast,
unspecified and metadata ranges are refused in both IPv4 and IPv6 forms. A
`webhook_url` on a scrape is delivered HMAC-SHA256 signed, retried on a bounded
ladder, and exhausted deliveries land in a dead-letter file.

Every request that reaches the service is appended to an append-only,
hash-chained usage ledger — refusals included, because a tenant who asked and
was declined still consumed the attempt. `GET /v1/usage` folds that ledger into
per-key, per-day and total rows; the console's **Usage** page reads the same
report.

Details: [`docs/api/26-managed-scrape-api.md`](docs/api/26-managed-scrape-api.md),
[`docs/modules/tenancy.md`](docs/modules/tenancy.md),
[`docs/modules/ssrf.md`](docs/modules/ssrf.md),
[`docs/modules/usage.md`](docs/modules/usage.md).

### Scheduler & run history

`harvest serve` repeats a config on `schedule.every_secs` (`0` = back-to-back)
for `schedule.runs` iterations (`0` = until stopped). Every finished run —
`run` and `serve` alike — appends one JSONL line to `schedule.history_path`:

```json
{"run":"catalog","ts":1789131958,"status":"ok","duration_ms":1027,
 "config_sha256":"6686...","targets":2,"respect_robots":true,
 "summary":{"pages_fetched":5,"records":5,"written":5,...}}
```

`config_sha256` is a stable hash of the serialized policy: an audit trail
proving *which* politeness settings produced *which* delivery — cheap
attestation for enterprise contracts (a client can diff it against the config
you signed off). Failed runs are recorded with `status:"error"`, and the
scheduler continues.

### Operations CLI (the API, from wherever you already are)

Every admin view has a CLI twin, so cron boxes and jump hosts don't need an
HTTP client: `harvest status` (run, policy hash, last run, storage lanes),
`harvest health` (offline lane check — or a live `/health` probe when an
admin address is configured; the exit code is the verdict), `harvest jobs`,
`harvest runs --status ok --sort duration --limit 50` (same bounded,
filtered, deterministic listings as `GET /runs`), `harvest workers|queue|
metrics` (live-only: they exit 1 with a clear message rather than show stale
zeros), `harvest proxies` (config view offline, EWMA health live),
`harvest datasets`, and `harvest config inspect|explain|migrate` (redacted — secret
fields print as `<redacted>`, provenance shows `explicit`/`default`/
`env:VAR`). `--json` gives pretty JSON, `--jsonl` streams one object per
line for `jq`, and `--url HOST:PORT` targets a non-default admin endpoint.
Data goes to stdout; logs and errors go to stderr, so redirected output is
always clean.

### Dead-letter queue (work you didn't get, and work to try again)

Any URL whose page request failed after retries (`failed:…`), was refused by
the site (`blocked:403`, …), or got robots-denied *on a retry*, is retained in
`dlq_path` as one JSON line. Entries include `{url, reason, attempts, target}`;
the file is rewritten atomically with owner-only permissions rather than
appended in place. Existing entries remain when a run is not retrying the DLQ.

For privacy, URL userinfo is removed (the URL is reduced to its origin) and
known-sensitive query values are masked. Such entries are marked
`"retryable": false` because the original request cannot safely be reconstructed;
configure credentials through trusted settings and review those failures
manually. Repeated failures are retained with `"dead_for_good": true` once
`dlq_max_attempts` is reached.

```bash
harvest run --config client.json            # broken pages accumulate in out/dlq.jsonl
harvest run --config client.json --retry-dlq  # also seeds the DLQ urls;
                                              # successes leave the file, failures
                                              # get attempts+1 and stay
```

Entries that hit `dlq_max_attempts` (default 3) are kept with a
`dead-for-good:<reason>` marker instead of looping forever. This is the
"guaranteed-later" story ops teams ask for: transient site failures don't
vanish, and replay is auditable (`attempts` counts).

## Distributed mode (Redis): shard a crawl across machines

Build with `--features redis`. One queue, N stateless workers — no shared
filesystem, no coordinator:

```bash
harvest push   --config big.json --redis redis://host:6379/0 --prefix job42 [--reset]
harvest worker --config big.json --prefix job42 --idle-secs 5   # run this on every box
harvest queue  --config big.json --prefix job42                 # jobs/inflight/dlq/dedup stats
```

* **Reliable queue**: `BRPOPLPUSH jobs→inflight`, ack on completion; a dead
  worker's job stays in `inflight` and `sweep_inflight` (run at cohort start)
  returns strays — nothing is silently lost. While a shard works a job it
  heartbeats a short-TTL lease naming itself (`--name`, default `w-<pid>`),
  so the sweep skips work that is slow-but-alive instead of double-processing
  it.
* **Shared politeness**: a Redis-time fractional token bucket, minimum
  spacing, robots `Crawl-delay` floor, and refusal backoff are shared atomically
  across workers — adding shards does not multiply the agreed request rate.
  robots.txt is fetched once per origin for the whole fleet (TTL-cached in
  Redis). Refusals (401/403/451) are never retried — retries are queue-level
  (`max_retries`) and land on `{prefix}:dlq` with a reason, same philosophy
  as the local dead-letter queue.
* **Cross-shard dedup**: start-URL enqueue and content fingerprints live in
  Redis sets, so racing workers can't write the same page twice.
* **Sinks**: each worker process writes its own `out.p<pid>.jsonl` fragment
  (`cat` merges); sqlite/postgres sinks are shared-safe as-is.
* Page budgets (`max_pages` global + per-target) are enforced via Redis
  counters (best-effort under races — the hard guarantee sites feel is the
  shared limiter).

Verified live: `tests/dist.rs` runs two concurrent shards against a real
redis-server + mock site (`HARVEST_TEST_REDIS=1`, asserts split-not-duplicated
results, shared pacing timing, fleet-single robots fetch, drain-to-zero, retry-
then-dead-letter, and inflight crash-recovery).

### Checkpoint / resume: crawls that survive anything

Set `"ckpt_path"` and long runs snapshot their discovery state (pending
queue, per-target visited URLs, fingerprints, budget counters) every
`ckpt_every_pages` records *or* `ckpt_every_secs` seconds, whichever comes
first. Writes are atomic (temp file + rename) — a kill mid-write can never
produce a torn checkpoint.

```bash
harvest run --config big.json            # interrupted? just restart the same way:
harvest run --config big.json --resume   # continues from ckpt_path; already-queued
                                         # URLs are NEVER re-fetched
```

The contract, verified live (SIGKILL at t=3.2s of a crawl-delayed site,
`tests/e2e.rs::checkpoint_resume_finishes_crawl_with_zero_refetch` + smoke):

* Resume **refuses** if the config's policy sha changed — a crawl finishes
  under the politeness rules it started under; audit history stays truthful.
* A **complete** run deletes the checkpoint (resuming then is a warned
  fresh start, not an error); an interrupted/failed run leaves one behind.
* JSONL/CSV sinks are append-mode, so the shard files from both runs form one
  continuous dataset. Anything the last checkpoint save covered is never
  re-fetched (fingerprint + visited are restored from it).
* Hard-kill costs at most one checkpoint-window of *queue* state, in both
  directions: a page whose response was in flight at kill time was already
  marked visited on enqueue, so resume skips it (the record is lost — visited
  knows it, the sink doesn't); a page the run *wrote* after the last save can
  be re-fetched, leaving one duplicate line in the append file. Both are
  bounded by tightening `ckpt_every_secs`; the DLQ remains the retry path for
  refusal-class losses in distributed mode. Deliberate at-most-once semantics
  at the crash boundary — documented and bounded, not an accident.
* Scheduler (`serve`) does not resume between runs (each run is a full
  crawl); checkpointing is `run`-scoped. Distributed mode (`worker`) doesn't
  need it — the Redis queue is the durable state.

### Incremental revalidation: repeat crawls pay 304s, not bodies

`"revalidate_store": "out/validators.jsonl"` turns every re-crawl (nightly
`serve` runs, manual re-runs) into a delta check. On a 200 the engine caches
the URL's `ETag`/`Last-Modified` **and the absolutized, filtered link set it
discovered from that page**. Next run, pages are requested with
`If-None-Match`; a 304 costs no body, no parse, no duplicate record — and
discovery still advances, replaying the cached links, so a page *added* to an
unchanged index is found anyway (verified by `tests/e2e.rs::revalidation_skips_bodies_but_not_the_crawl`
and a two-generation live smoke against the demo mock).

Rules the feature plays by:

* Politeness is never skipped — every revalidation is a paced, robots-gated
  request; we save bandwidth, not favors.
* `robots.txt` and sitemaps are *never* revalidated: policy and discovery
  must always be re-read (they are exactly what can change silently).
* `revalidate_ttl_secs` optionally forces a full refresh of stale entries
  (0 = trust validators until the site says otherwise).
* SLO math counts revalidations as delivered data (`written + revalidated`
  vs `min_records`) — a steady-state site is healthy, not an outage;
  `alerts::no_data` will not page you for "nothing changed".
* The validators file is atomic JSONL; unreadable file => feature disabled
  with a warning, never a failed crawl. Entries that could never answer 304
  are pruned on save.

### Client delivery: `harvest export`

```bash
harvest export --from out/results.jsonl --to deliver/ --history out/runs.jsonl
#               [--from more.jsonl ...]  [--split-target]
```

Produces a folder a client can trust without you on the phone:

* `records.csv` — meta columns (`run,target,url,depth,ts,fingerprint,egress`)
  lead, payload columns follow in sorted order; nested `fields` objects
  dot-flatten (`meta.stock`), arrays render as compact JSON in RFC-4180
  quoting. **Deterministic**: the same input bytes always produce the same
  file bytes — diffs of re-deliveries are meaningful.
* `records.jsonl` — canonical re-serialized records (or one pair per target
  with `--split-target`: `catalog.csv`, `api.csv`, …).
* `manifest.json` — record/unique-url/skipped-line counts, per-target totals,
  column list, **sha256 + bytes + lines for every file**, and — given
  `--history` — the `config_sha256` of the delivered run(s): the client can
  verify what they got (recompute) *and* prove which politeness policy
  produced it. Unparsable input lines are counted, never silently dropped.

S3/GCS/SFTP upload stays external by design (`rclone sync deliver/ s3://…`)
— the engine ships data, not cloud credentials. Parquet on request: duckdb
`read_jsonauto('records.jsonl')` converts losslessly; in-process arrow is a
weight the binary refuses to carry.

### Politeness audit: `audit.json` receipts

`"audit_path": "out/audit.json"` (top level) makes every run record what it
actually *sent*, per host: request counts by status (200 / 304 / 404 / any
refusal, tallied by code), transport errors, how many politeness waits slept
and for how long, crawler-side backoff applied after refusals, abandoned-host
breaker trips, and robots.txt facts (fetched once? present? crawl-delay and
the number of URLs we dropped without ever requesting them). Records written
per host are in the ledger too, so a data-ops team can audit *output* against
*traffic* without opening the raw JSONL.

The report embeds the run name, the wall-clock window, the effective pacing
policy per host, and the same `policy_sha256` that `harvest export` writes —
one hash links receipts and delivery. `harvest audit --file out/audit.json
--md` renders it as a table for tickets and postmortems.

**It reconciles.** Every byte the engine put on the wire for that host appears
in the ledger — including robots.txt and sitemap fetches — so `requests +
robots_fetches` equals the hit count in the *site's own access log* for your
user-agent. That's the property that makes it usable as evidence in a
"you hammered us" dispute, and an e2e test asserts it against a counting mock
server (tests/e2e.rs `audit_report_reconciles_with_server_log`).

### Attestation: `harvest attest`

Evidence without integrity is folklore. Set `"attest_key_path": "out/attest.key"`
(generate one with `harvest attest --gen-key out/attest.key` — 32 fresh bytes,
mode 0600, never overwrites) and the engine signs every `audit.json` it
writes; `harvest export --key out/attest.key` signs the delivery `manifest.json`.
Signatures are **detached** `<file>.sig` files holding one hex HMAC-SHA256
over the artifact's exact bytes — the signed files are never rewritten, so
their sha256s and rclone mirrors stay stable.

```bash
harvest attest --verify out/audit.json --key out/attest.key
# OK: out/audit.json is intact and was signed by a holder of out/attest.key
```

A flipped bit anywhere in the receipt — one changed count, one removed
denial — flips the exit code and prints `HMAC verification FAILED`. Because
the manifest lists the sha256 of every payload file, one manifest signature
transitively attests the whole delivery folder. Anyone holding the shared key
can verify without trusting the sender's word; the test suite pins the HMAC
against RFC 4231 vectors and an e2e proves the engine's signed receipt
detects a single-bit edit. (As with any symmetric scheme: the key proves
third-party tampering, and both sides can produce it — that is the trade
that keeps this out of certificate-management hell. Move to asymmetric
signing only if your contract demands non-repudiation.)

## Observability

`GET :9678/` (self-contained dark-mode dashboard: live counters, proxy EWMA
health, recent-runs table with policy hashes — zero external assets so it
works inside air-gapped client VPCs), `/metrics` (Prometheus text), `/stats`
(JSON), `/runtime` (engine runtime snapshot: job accounting, scheduler
queue, worker states, open circuit breakers, event-bus health), `/events`
(structured-event tail, `?limit=`), `/recent` (history tail), `/health`,
`/policy` (machine-readable stance — hand this to a client's legal team),
plus the operational views — `/jobs`, `/runs`, `/failures`, `/retries`,
`/workers`, `/queue`, `/proxies`, `/pipeline`, `/storage`, `/diagnostics`,
`/alerts`, `/ops`, `/audit`, `/config` (all bounded and filtered; six POST
mutations: `/alerts/ack`, `/ops/note`, `/control/crawl`, `/control/export`,
`/control/ai`, `/control/jobs/:id/cancel`). `/health` and `/live` are the
unauthenticated liveness routes. Every response carries an `x-request-id`
correlation id; structured API errors use one JSON envelope, while unknown-route
404 hints are plain text and artifact downloads return file bytes — see
[docs/api.md](docs/api.md). Counters include
what a compliance team actually asks for: `harvest_robots_denials`,
`harvest_politeness_sleeps`, `harvest_requests_blocked`,
`harvest_backoff_sleep_ms_total`, `harvest_domains_abandoned`.

## Architecture

One process, one pipeline. Each box is a module you can open (names in
[docs/codebase-map.md](docs/codebase-map.md)); nothing in the diagram exists
in docs but not in the tree.

```
 config.json ─► Engine (bounded workers)
                 │  per-domain Politeness (token bucket + min-interval + breaker)
                 │  RobotsCache (TTL)        ProxyPool (EWMA health → failover/geo)
                 ▼
        Fetcher (reqwest/rustls, honest UA, stateless, retry+jitter)
                 ▼
        Extract (CSS / JSONPath-lite) → fingerprint dedup
                 ▼
        Sink (JSONL · CSV · SQLite · Postgres · dataset-dir · HTTP)
                 ▼
        admin server (:9678) → Prometheus → Grafana (ops/)
```

```mermaid
flowchart LR
  W[Console (web/ React)] -->|fetch /api/*| A["admin API<br/>src/admin.rs (28 GET + 6 POST)"]
  A --> E[Engine<br/>src/engine.rs]
  E --> S[Scheduler<br/>src/sched.rs + history]
  S --> P["Policy + Robots<br/>policy.rs · robots.rs · budget.rs · breaker.rs"]
  P --> F[Fetcher<br/>fetcher.rs + retry.rs + reval.rs]
  F --> X[Proxy pool<br/>proxy.rs]
  X --> CP["Crawl planning + discovery<br/>crawl/mod.rs · crawl/discovery.rs · content.rs"]
  CP --> EX[Extract<br/>extract.rs jsonpath/css]
  EX --> AI["Optional AI assist<br/>ai.rs (off by default, never authoritative)"]
  AI --> T[Transform<br/>transform.rs typed ops]
  T --> SC[Schema<br/>schema.rs enforce + drift]
  SC --> D[Dedupe<br/>ids.rs fingerprint]
  D --> K[Sink<br/>sink.rs jsonl/sqlite/pg + quarantine]
  K --> EXP["Export + attest<br/>csv · jsonl · xlsx · pdf · parquet · xml · zip · sign.rs"]
  K --> AUD[(Audit<br/>audit.rs hash-chained)]
  E --> M["Metrics + events<br/>metrics.rs · events.rs"]
  S <-.->|--features redis| RQ[(Redis queue<br/>exec.rs)] --> WR[Workers<br/>supervisor.rs]
  W -.->|polls every 5–10s| A
```

Every arrow is code: Web→API is the typed client (`web/src/lib/api.ts`),
API→Engine is the in-process router, and the rest is the run loop in
`src/runner.rs`. DEMO vs LIVE: the console only shows engine-reported values;
`VITE_MOCK=1` dev mode is banner-marked and compiled out of releases.

### Terminology (CLI ↔ Web parity)

| Term | CLI surface | Web surface | Module |
|---|---|---|---|
| target | config `targets[]` | Config Studio → Target Builder | `src/runner.rs` |
| run | `harvest run` / `serve` | `/runs` + Run drawer | `src/engine.rs` |
| job | `harvest jobs` | `/jobs` | `src/job.rs` |
| worker | `harvest workers` | `/workers` | `src/supervisor.rs` |
| proxy / egress | config `proxies[]` | `/proxies` | `src/proxy.rs` |
| policy | `harvest schema` prints its sha | Overview policy chip | `src/policy.rs` |
| schema | `harvest schema --config …` | Quality view | `src/schema.rs` |
| sink / quarantine | `out/*.jsonl` files | `/storage`, Quality | `src/sink.rs` |
| audit | `harvest audit` | `/audit` | `src/audit.rs` |
| export / verify | `harvest export` + `verify` | API Explorer docs link | `src/export.rs` |

The same nouns mean the same things everywhere; docs never call a run a
"task" or a proxy an "endpoint".

## Development

```bash
# the gates every commit must pass (see .github/workflows/ci.yml):
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features  # live Redis/Postgres tests are env-gated; full live-service run: 728 passed
cargo build --release --all-features

# feature combos (CI checks each in isolation):
cargo check --all-targets                       # default: no optional backends
cargo check --all-targets --features sqlite
cargo check --all-targets --features postgres
cargo check --all-targets --features redis

# live-infra suites — GATED: they skip cleanly (and print why) unless the
# env var is set, so `cargo test` needs no services by default:
HARVEST_TEST_REDIS=1 cargo test --features redis --test dist   # wants redis on 127.0.0.1:6399 (db 9)
      # override with HARVEST_TEST_REDIS_URL=redis://host:port/db
HARVEST_TEST_PG=1    cargo test --features "postgres" --test pg  # wants a real PostgreSQL
      # DSN via HARVEST_PG_DSN (default: host=127.0.0.1 port=5432 user=harvest password=harvest dbname=harvest)
      # (the live suites are part of the 83 and skip with a printed notice
      # when their env gate is unset — CI core jobs need no services)
```

## Project structure

```
harvest/
├── src/                  70 source files, one concern each (lib layout in docs/architecture.md)
│   ├── main.rs           CLI (clap) — thin plumbing over the library
│   ├── config.rs         all fields + defaults, ${VAR} injection, validate()
│   ├── engine.rs         the crawl loop          policy.rs  robots.rs
│   ├── fetcher.rs        one HTTP request       proxy.rs   extract.rs
│   ├── sink.rs pgsink.rs export.rs               ckpt.rs  reval.rs
│   ├── audit.rs sign.rs  metrics.rs  alerts.rs   admin.rs  runner.rs
│   ├── crawl/            planner (mod.rs) + discovery.rs — pure, bounded, testable
│   ├── content.rs        MIME/charset classification, size limits, safe decoding
│   ├── document.rs media.rs   typed document + resource metadata (no payloads)
│   ├── ai.rs ai/         optional assistance: budgeted, redacted, fail-closed
│   ├── tenancy/          tenants, digest-stored keys, scopes, per-key limits
│   ├── ratelimit.rs      per-key token bucket + concurrency cap + monthly quota
│   ├── ssrf.rs           loopback/private/metadata refusal, every redirect hop
│   ├── usage/            hash-chained ledger, windowed aggregation, export
│   ├── scrape_api/       the /v1/* handlers, jobs, idempotency, webhooks, OpenAPI
│   ├── export/           xlsx.rs · pdf.rs · parquet.rs · xml.rs · bundle.rs (ZIP)
│   └── dist.rs           redis queue (feature-gated)
├── tests/                33 files: e2e.rs · security.rs · dist.rs · pg.rs · crawl_discovery.rs ·
│                         ai_pipeline.rs · export_formats.rs · content_integrity.rs ·
│                         managed_api.rs · tenancy_security.rs · openapi_contract.rs
├── examples/             basic / production / distributed configs + api-requests.sh
│   └── output/           real smoke-run artifacts (records, signed audit, signed manifest)
├── demo/                 self-contained mock site (server.py) for offline demos
├── ops/                  docker-compose.yml · prometheus.yml · grafana provisioning
├── docs/                 architecture · api · configuration · deployment ·
│                         troubleshooting · development
├── Dockerfile            multi-stage, distroless-ish runtime, non-root, HEALTHCHECK
├── SECURITY.md           threat model, enforced guarantees, production checklist
└── CHANGELOG.md          version history (semver, real entries only)
```

## Technical highlights

* **Zero network in tests** — every e2e test runs against a loopback mock
  spawned by the test itself; CI core jobs need no services and no secrets.
* **Bounded everything**: body size (declared + streamed, checked twice),
  redirects (≤ 5), retries (≤ 1 + `max_retries`, exponential + jitter, each
  wait ≤ `max_single_wait_ms`), concurrency (≤ 512), proxies (≤ 256),
  sitemap seeds, `max_pages` per target and globally.
* **Fail-closed startup**: 25+ validation rules; an unsafe config exits 1
  with the rule name and the remedy instead of running unsafely.
* **Secret hygiene by construction**: `${VAR}` whole-value injection,
  missing-var = named error, credential-bearing URLs redacted at every log
  site, attestation hash computed pre-expansion so secrets never leak into
  the recorded policy identity.
* **Hand-rolled admin server** (no framework): 8 KiB header cap, 2 KiB query
  cap, bounded per-route POST bodies (4 KiB operator actions / 64 KiB control
  commands), method+path routing over 50 routes (35 GET + 15 POST, of which 16
  are the `/v1/*` customer + operator surface), timing-safe
  bearer comparison, constant 401 bodies, and `nosniff`/`no-store` headers.
* **Deterministic exports**: same inputs ⇒ byte-identical CSV/JSONL
  (only `generated_ts` moves); manifest hashes every file; HMAC receipts are
  one `sha256sum`-level dep away from client-verifiable (no PKI needed).
* **Resume that refuses**: checkpoints carry the policy hash; changed rules
  mid-crawl ⇒ resume rejected, because the crawl may have promised different
  pacing to the target.
* **Poison-tolerant locks**: a panicking task can't wedge a day-long run.

## Known limitations

Stated because they matter to deployment decisions — none is a hidden risk:

* Shutdown handles **SIGINT and SIGTERM**. Requests and storage waits cancel,
  owned delivery threads drain/join, and terminal jobs do not leave background
  delivery workers running. Allow at least 45s of container/service stop grace.
* Attestation is **symmetric HMAC** — strong tamper-evidence, but both
  key-holders can produce a valid signature (asymmetric signing is Roadmap).
* HTML parsing is static (`scraper`): **no JavaScript rendering**; pages that
  render behind JS must be captured via their JSON endpoints (`mode: json`).
* `include`/`exclude` are **substring** rules on the absolute URL (cheap,
  auditable — not regex).
* Sitemap support is `<urlset>` + one level of `<sitemapindex>` (nested
  indexes beyond depth 1 are skipped by design, with a warn line).
* Redis mode requires a binary built `--features redis`; the public image
  ships sqlite+postgres (rebuild with the feature for queue mode).
* No built-in TLS listener: terminate TLS at a reverse proxy (the engine
  speaks plain HTTP to `metrics_addr`; upstream HTTPS is honored).
* Postgres sink targets one table (`harvest_records`, auto-created); schema
  evolution/partitioning of huge tables is a DBA-side concern.
* Pacing implementations differ by mode: single-process uses an in-memory
  token bucket; distributed mode uses an atomic Redis-time token bucket with
  shared minimum spacing, crawl-delay, and refusal backoff per Redis prefix.
  Workers under different prefixes do not coordinate.

## Production checklist (buy-side summary)

Full text: [SECURITY.md](SECURITY.md) §checklist and
[docs/deployment.md](docs/deployment.md). One screen, every item verified by
engine behavior or the compose file as shipped:

- [ ] `harvest validate` passes on the deploy-time config (all rules, offline)
- [ ] secrets only via `${VAR}`; compose `--env-file`/host env; none in git
- [ ] `admin_token` (≥ 12 chars) set whenever `metrics_addr` is not loopback
      — open-bind-without-token fails startup, by design
- [ ] `rediss://` for remote Redis, `sslmode=require` for Postgres
- [ ] container non-root (uid 10001), `read_only: true`, `cap_drop: [ALL]`,
      single writable volume `chown`ed to 10001
- [ ] HEALTHCHECK on `/health` (auth-exempt by design); orchestrator uses it
- [ ] logs → stdout only, rotated by the runtime; RUST_LOG left at info
- [ ] backups: history file, per-delivery audit.json(.sig), attest key
      archive, `pg_dump` for Postgres sinks
- [ ] `stop_signal: SIGTERM` + 45-second grace period for drain-on-stop
- [ ] Prometheus scrape carries `$HARVEST_ADMIN_TOKEN` bearer (compose wires it)

## Project status

Version **0.10.0** (see CHANGELOG). Licensed MIT (see [LICENSE](LICENSE)) —
the `license` field in `Cargo.toml`; adjust terms before commercial
distribution. Verified here: **728 Rust tests** in the all-target/all-feature
run with live Redis and PostgreSQL; **394 console tests across 129 files**;
ESLint, TypeScript, production build, and npm audit pass; buyer-flow E2E is
**18/18**. All four backend feature configurations compile-check; clippy
`-D warnings` and rustfmt pass; 302 repository links checked with 0 broken.
The
engine is pre-1.0: config field names and file formats are treated as the
compatibility surface and any break will be changelogged with a migration
note. This README, `docs/`, `examples/` and `SECURITY.md` describe the
behavioral contract of the binary built from this tree — anything not
documented here or in `docs/` is not a promise the engine makes.

## Web console

`web/` ships a premium React + TypeScript operations console for the admin
API — 29 pages (overview, runs, jobs, queue, workers, command center, run
detail, proxies, activity, pipeline, quality, events, storage, alerts, audit,
diagnostics, delivery verify, config studio, config diff, API explorer, API
keys, usage, export center, crawl studio, browser studio, AI assist, control
jobs, control job, artifacts), all
bound to routes the engine serves or to explicitly local-only tooling (the
two config tools and the browser-side digest check touch no invented backend). First run shows a
welcome panel built from live state and an optional (skippable, never
blocking) product tour. No config-write is faked: the
studio edits an in-memory draft with server-stripped export for
`harvest validate`. Bearer token lives in memory only; secret fields stay
redacted; dev needs nothing but `npm run dev` (vite proxies `/api` →
`127.0.0.1:9678`). Full contract: [docs/web-client.md](docs/web-client.md).

```sh
cd web && npm install && npm run dev      # http://localhost:5173
npm run typecheck && npm run lint && npm test && npm run build
npm run smoke                             # built-dist + live-API e2e smoke (needs `harvest serve`)
npm run e2e:buyer                         # full buyer flow, self-managed backends
```

### Size, measured (no performance promises, just this build)

From `npm run build` on this tree (Vite production output; `dist/` = index.html + 45 asset files):

| artifact | bytes | gzip |
|---|---|---|
| index.html shell | 546 | — |
| entry chunk `index-*.js` | 50,232 | 16,892 |
| vendor chunk `vendor-*.js` | 140,726 | 45,096 |
| stylesheet `index-*.css` | 37,215 | 8,071 |
| initial-load JS+CSS total | 228,173 | 70,059 |
| whole `dist/` (all lazy route chunks) | 423,362 | — |

Every route is its own lazy chunk (29 pages + shared DataTable/MetricCard
chunks); first paint pulls only the three initial assets above. Perf work is
deliberately un-done beyond this: the budget here is honesty — if a future
change moves these numbers, the table above is stale and must be updated by
measurement, not aspiration.

## Roadmap (things a paying client will ask for next)

- S3 *upload* on top of the dataset sink (or just rclone-sync the dir)
- non-repudiable attestation: asymmetric signatures over `audit.json` /
  `manifest.json` (today's `harvest attest` is symmetric HMAC — strong
  tamper-evidence, but both key-holders could produce a valid signature)
- per-file `.sig`s in the export folder (currently one signature over the
  manifest, which itself hashes every payload file)
