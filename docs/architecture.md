# Architecture

`harvest` is a single statically-linked binary plus a small set of
sidecar-free integrations (Postgres, Redis are optional, feature-gated).
Everything below describes code that exists in `src/`; no planned
components are documented.

## System view

```
┌───────────────────────────────────────────────────────────────────────────┐
│ CLI (src/main.rs): run · serve · history · validate · fetch · push ·      │
│ worker · queue · export · audit · attest                                  │
└─────────────┬─────────────────────────────────────────────────────────────┘
              ▼
┌─────────────────────────────┐   ┌──────────────────────────────────────┐
│ Config (src/config.rs)      │   │ Runner (src/runner.rs)               │
│ serde+validate, ${VAR}      │◄──┤ serve(): repeat loop, run history    │
│ expansion, policy sha256    │   │ (schedule.history_path, newest wins) │
└─────────────┬───────────────┘   └──────────────────────────────────────┘
              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│ Engine (src/engine.rs)                                                   │
│ bounded worker pool · per-run in-process queue (jobs {url, depth,        │
│ target}) · global/target page budgets · discovery · fingerprint dedup   │
│                                                                         │
│  politeness (src/policy.rs)      robots cache (src/robots.rs + TTL)     │
│  token bucket + min-interval     full_disallow / crawl-delay floor      │
│  + Retry-After backoff           per-origin cache                       │
│  + per-host circuit breaker      (shared with seed + sitemap gates)     │
│                                                                         │
│  fetcher (src/fetcher.rs)        proxy pool (src/proxy.rs)              │
│  1 reqwest client per egress     static from config, EWMA health,      │
│  timeouts, ≤5 redirects,         weighted pick, failover scoring,      │
│  body cap, retry/backoff         no growth at runtime                   │
│  jitter loop                                                   │        │
│         ▼                                                        ▼       │
│  extraction (src/extract.rs)                       sitemap (src/sitemap.rs)
│  CSS rules (html) / JSONPath-lite (json)           <urlset> + index,     │
│  fingerprint = sha256(normalized body text)        robots-prefiltered    │
│      ▼                                                                    │
│  pipeline (src/transform.rs + src/schema.rs): per-field op chains, then  │
│  the declared-schema lane — coerce what can be coerced, quarantine the   │
│  rest to its own JSONL; identity for every legacy config (zero cost)     │
└─────────────┬───────────────────────────────────────────────────────────┘
              ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Sinks (src/sink.rs, src/pgsink.rs): jsonl · csv · sqlite · postgres ·    │
│ dataset(dir/dt=YYYY-MM-DD/) · http(batched POST) · null                  │
│ side-effects: checkpoint (src/ckpt.rs) · revalidation store (src/reval.rs)│
│ politeness audit (src/audit.rs) · detached HMAC signatures (src/sign.rs)  │
│ client delivery export (src/export.rs)                                    │
└─────────────┬────────────────────────────────────────────────────────────┘
              ▼
┌──────────────────────────────────────────────────────────────────────────┐
│ Admin (src/admin.rs) on metrics_addr: dashboard /, /metrics (Prometheus   │
│ text), /stats, /recent, /policy; /health auth-exempt liveness.            │
│ Metrics counters live in src/metrics.rs (shared Arc) — sink/queue code     │
│ only bumps counters, never vice versa.                                      │
│ Alerts (src/alerts.rs): SLO evaluation per run → webhook POST (redacted).   │
│ Distributed queue (src/dist.rs, `--features redis`): push / worker / queue │
└──────────────────────────────────────────────────────────────────────────┘
```

Only components in this diagram exist. Deliberately absent (and rejected at
config time or by design): TLS/JA3 fingerprint spoofing, anti-bot evasion,
proxy rotation to reset a host refusal, CAPTCHA handling, browser execution.

## Module responsibilities & boundaries

| Module | Owns | Never does |
|---|---|---|
| `config` | schema, defaults, `validate()`, policy hash (`hash()`), `${VAR}` env expansion, credential redactors | network, IO besides reading its file |
| `engine` | crawl loop: workers, dedup, discovery gating, record construction, outcome classification, side-effect wiring | writing SQL directly (delegates to sinks), HTTP parsing details (fetcher), queue internals (sched), retry math (retry) |
| `job` | job identity, lifecycle state machine (`Pending→Running→terminal`, illegal moves are hard errors), deadlines, retry metadata, structured status | owning the queue (sched) or the loop (engine) |
| `sched` | the crawl queue: fifo/priority/fair modes, bounded depth (backpressure = refused intake), per-target inflight gates, deadline enforcement at pop, queue stats | evicting queued work, ever; blocking workers on full |
| `supervisor` | worker registry/health states, restart policy with backoff, cancel-token drain, bounded `TaskSet` for background tasks | resurrecting a task beyond `worker_restarts`; changing first-death semantics when the policy is 0 (legacy) |
| `exec` | the request envelope: `RequestPlan` (deadline, caps), `RequestContext` (correlation id, bus/accounting/budget handles), `ExecutionMeta` (attempts, timing, egress) | owning transport rules (fetcher) or policy math (retry) |
| `retry` | the retry decision: reason taxonomy, the delay curve (byte-identical to the shipped one), exhaustion policy, per-minute budget, per-target ledger, deterministic test mode | making HTTP calls |
| `breaker` | per-host transport circuit: closed/open/half-open, single-probe half-open, injected-clock transitions, metrics | counting refusals toward opening (401/403/429/451 never trip it) |
| `events` | bounded event bus: ring with drop-oldest + `dropped` accounting, `next_after` rejoin-on-gap, close-releases-waiters, JSONL via `events_path` | unbounded buffering; losing events silently |
| `accounting` | run-scoped resource counters (jobs by state, retries, bytes, queue waits), failure ring (64, redacted URLs), retry-delay history, throughput math | blocking writers: everything is an atomic add or a short-mutex ring |
| `runtime` | owns bus + accounting + breakers + supervisor/queue handles; renders the one `GET /runtime` snapshot; hosts the events-log writer contract | control authority — it observes; the engine decides |
| `policy` | per-host token bucket, min-interval, `Crawl-delay` floor (only tightens), `Retry-After` backoff, block-breaker counts | relaxing a site's stated pacing, ever |
| `fetcher` | one `reqwest::Client` per egress, timeouts, ≤5 redirects, bounded body read, retry+jitter loop, `FetchError` taxonomy | persisting anything |
| `proxy` | static pool from config; EWMA latency+success scoring; weighted pick; report(ok/fail) | growing at runtime; hiding refusals (refusal is scored neutral to the *proxy*, hostile to the *host breaker*) |
| `robots` | RFC-9309-ish parser: groups by UA substring, longest match wins, `Crawl-delay`, `Sitemap:` lines, `present` flag | caching beyond `robots_ttl_secs` |
| `sitemap` | `<urlset>` / `<sitemapindex>` scan; entity/CDATA tolerance | resolving relative locs (engine does, then origin+robots filter) |
| `extract` | selector compile+select (html mode), JSONPath-lite (`$.a[*].b`), canonical fingerprint | network |
| `sink` | `Sink` trait; jsonl/csv (header-once), sqlite (feature), dataset (per-day files), http (batches), null | reading config mid-run (all params injected at open) |
| `pgsink` | `harvest_records` DDL (created if missing), batched INSERT `ON CONFLICT (run,target,fingerprint) DO NOTHING` | leaking DSN into errors beyond its own context strings |
| `dist` | Redis job/inflight/dlq lists + dedup sets + fixed-window per-host limiter + idle drain + sweep | owning the sink (each worker writes its own fragment) |
| `ckpt` | atomic checkpoint file (tmp+rename), policy-sha drift refusal | auto-resuming (resume is operator-intent via `--resume`) |
| `reval` | per-URL validators (etag/last-modified) + cached link sets (cap 500) + TTL condition | storing bodies |
| `audit` | in-memory per-host recorder → atomic `audit.json` snapshot incl. totals + policy echo | sampling — every engine send is counted |
| `export` | JSONL → wide CSV (dot-flattened fields) + canonical JSONL + sha256 `manifest.json`; optional history→policy-hash link; optional HMAC `.sig` | network; S3 (delegated to rclone) |
| `sign` | HMAC-SHA256 key file handling (0600, refuse-overwrite), detached `<file>.sig`, constant-time verify | storing anything signed itself |
| `admin` | tiny HTTP/1.1 responder (raw tokio socket, no framework), bearer-gated routes, zero-asset dashboard | serving anything from disk beyond `history_path` reads |
| `metrics` | `Arc`-shared atomic counters; Prometheus text render | labels with unbounded value sets (labels are per-target ids from config + fixed names) |
| `alerts` | evaluate SLOs on the run's `Summary` → single webhook POST; `no_data` counts written+revalidated | logging webhook URL paths (origin only) |
| `runner` | serve loop, `record_history`, stop-flag handling, alert dispatch | modifying config between runs (fresh `Config` per run is the operator's job) |
| `robots`+`engine` seed path | start-URL seeding, sitemap seeding, dlq re-seeding, `fetch` one-shot | queueing URLs that fail include/exclude, same-origin, or robots checks |

## Request lifecycle (one job)

1. `WorkerCtx::process(job)` → `policy.is_abandoned` gate (breaker → audit
   `note_abandoned`, job dropped).
2. `policy.wait(host)` — token bucket + min-interval, with the site's
   `Crawl-delay` as a floor; engine times the sleep and records it in the
   audit ledger (`note_wait`).
3. robots gate for the origin (TTL-cached; cached misses still count once),
   crawl-delay re-applied via `note_crawl_delay`.
4. Conditional-GET headers attached if `revalidate_store` has validators for
   this URL (unless TTL-expired).
4b. transport breaker gate (only with `transport_breaker`): `Open` → job
    retired with `breaker:open`; `Probe` → the single half-open test is
    this request; a job ending without a fetch releases the probe slot.
5. `fetcher.execute(plan, ctx)` — pick egress (proxy pool weighted),
   bounded body read, retry decisions delegated to `retry::RetryPolicy`
   (same curve as before; plus deadline check + budget gate + events),
   then classify:
   - `200/other` → outcome; `304` → revalidated path (no body, no record,
     cached link graph replayed so discovery continues); `401/403/429/451`
     → `Blocked` (metrics, per-target counter, DLQ note, audit
     `note_request+note_backoff`); `5xx`/transport → `Failed` after
     ≤`max_retries` capped-backoff attempts; size-cap refusals →
     `Permanent` (never retried).
6. Extract per target rules → build `Record` with meta fields →
   fingerprint dedup (content-hash, per target) → `Sink::write` →
   audit `note_written` → link discovery: same-origin, include/exclude,
   robots, `max_depth`, visited-set, then enqueue children.
7. Counters update (`metrics`) at every stage boundary; the admin API and
   Prometheus text are pure reads of the same atomics.

## Retry / backoff / failover semantics

- Retry math lives **only** in `retry::RetryPolicy` (used by the fetcher's
  engine path; the legacy `get`/`get_with` loop is kept for robots/sitemap
  one-shots and is formula-identical): exponential
  `retry_base_ms·2^attempt` + pseudo-jitter, capped per-sleep by
  `max_single_wait_ms`, count capped by `max_retries` (validate ceiling 10).
  `Blocked` and `Permanent` are never retried; `5xx` and transport errors
  are. The engine path adds, without changing the curve: deadline
  enforcement (an attempt that cannot finish in time is not started), the
  shared `retry_budget_per_min` token bucket (denied retries surface as
  `retry_budget_denied`), and per-attempt events. `retry_deterministic`
  zeroes jitter for exact reproduction.
- A refused response additionally arms the **politeness** layer
  (`policy.note_block`: honors `Retry-After` clamped by config
  `block_backoff_secs`, repeated blocks escalate; `block_breaker_count`
  consecutive blocks abandon the host for the run).
- Proxy failover is statistical, not punitive: fetch failures and streamed
  body-read failures report `ok=false` (draining that egress's weight);
  refusals report `ok=true` (the proxy did its job; the site said no — and
  rotating egress does **not** reset the host breaker, by design).
- Post-run leftovers: with `dlq_path`, failed URLs are appended as
  `{"url","reason","attempts"}`; `harvest run --retry-dlq` re-seeds them,
  clearing successes and capping dead-for-good at `dlq_max_attempts`.

## Engine-layer event & snapshot flow

The engine owns the control flow; the engine layer owns its *shape*:
`job` defines what a unit of work is, `sched` where it waits, `supervisor`
who runs it and what happens when they die, `exec`/`retry`/`breaker` how a
request is planned, retried and short-circuited, `events`/`accounting`/
`runtime` what remains observable afterwards. Every integration point is
default-off or default-legacy: with a stock config the FIFO queue is the
old `VecDeque`, the restart policy is fail-first-death, there are no
deadlines, no budget, no breaker — while the event ring and accounting run
at atomic cost (a bounded ring, ~1–2 MB ceiling) because observability that
can be switched off is observability that was off when you needed it.

## Checkpoint lifecycle (only with `ckpt_path`)

- At run start (with `--resume`): file read → schema/version/run-name checks →
  `policy_sha256` must equal the current config hash or resume is refused
  (a changed policy would violate the rules the crawl started under).
- During the run: every `ckpt_every_secs` or `ckpt_every_pages` (first
  trigger) an atomic snapshot (tmp+rename, `fsync`-ish replace) captures
  pending queue + per-target visited/fp/written state.
- At completion: checkpoint file is **deleted** (a finished crawl has no
  resumable state; a stale file must not haunt the next run).
- Crash (`kill -9`, host down): the last snapshot survives;
  `harvest run --resume` continues with zero refetch of completed pages
  (proven by tests, including a live SIGKILL smoke).

## Worker lifecycle — in-process and distributed

- In-process: `concurrency` tokio tasks pull from one bounded queue; each
  ends when the queue drains against the page budgets; shutdown: SIGINT sets
  a stop flag checked at loop heads — in-flight jobs complete (drain),
  queue drops, sink flushes, artifacts (history/audit/ckpt-delete) write.
- Distributed (`--features redis`): `harvest push` seeds
  `{prefix}:jobs` with start URLs (and optional sitemap seeds); each
  `harvest worker` runs the same politeness/robots/extract stack but pulls
  jobs via `BRPOPLPUSH jobs inflight` (the timeout only bounds the wait —
  there is no lease expiry), acks by exact-token `LREM`, dedups via shared
  sets, and enqueues discovered children back to `{prefix}:jobs`. A job a
  killed worker held stays in `inflight` until an operator calls
  `Queue::sweep_inflight` (no live workers on the prefix first). Workers exit when the queue stays empty
  for `--idle-secs`. Each shard writes its own sink fragment (`.p<pid>`
  suffix for file sinks); `harvest queue` prints job/inflight/dlq/dedup
  stats for operators.

## Storage abstraction

`Sink` is a two-method trait (`write`, `flush`). `sink::build(&SinkCfg)` maps
config to implementation; all sinks accept the same record shape, so export,
audit reconciliation, and dataset partitioning depend only on that shape.
Postgres is compile-time optional: `SinkCfg::Postgres` exists in the schema
regardless, but `validate()` bails with "binary lacks `--features
postgres`" when not built in. Redis behaves the same for `push/worker/queue`.

## Observability flow

`Metrics` atomics ← every engine/sink/fetcher stage; consumed three ways:
Prometheus text endpoint, `/stats` JSON (adds admin liveness + proxy pool
snapshot), and the run `Summary` (per-run totals) that the runner appends to
`schedule.history_path` (one JSON line per run: run, ts, status, duration,
config_sha256, targets, respect_robots, summary) and feeds `alerts.rs` SLO
evaluation. The politeness `audit.json` is a **separate** ledger written only
when `audit_path` is set: per-host send counts (including robots/sitemap
fetches — the property that makes it reconcile 1:1 with a site's access log),
waited/backoff milliseconds, denials, and records written.

## Error propagation

- Startup: any `validate()` rejection is fatal with an actionable message —
  silent fallback is not allowed.
- Per-run: worker task panics surface via join results as
  `worker task died: …`; the first error aborts the run after drain and the
  serve loop records `status:"error"` history lines but keeps scheduling.
- Sink/IO errors bubble as `anyhow` chains with context at each layer
  (`write <path>`, `read <path>` …). Mutex poisoning is recovered
  (`into_inner`) at every crawl-path lock so one panic cannot brick every
  later lock in a long run.
- `FetchError` is the typed network vocabulary (`Blocked`, `RobotsDenied`,
  `Failed`, `Permanent`); engine maps each to distinct metrics + counters,
  never to a stringly-typed catch-all.

## Feature flags

| Cargo feature | Enables | Absent behavior |
|---|---|---|
| `sqlite` | `rusqlite` sink | `kind:"sqlite"` → startup error naming the feature |
| `postgres` | `tokio-postgres` sink (`pgsink`) | same pattern |
| `redis` | `dist.rs`, `push`/`worker`/`queue` subcommands (clap-gated) | subcommands do not exist in `--help` |
| (default = none of the above) | pure-std+tokio+reqwest build | — |

Docs also live in the code: `cargo doc` surfaces module-level rustdoc, and
every `SinkCfg` variant carries its shape comment.
