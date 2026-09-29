# Development guide

Everything here is a command that works on a fresh clone on Linux; CI
(`.github/workflows/ci.yml`) runs the same set. No codegen, no build scripts,
no non-crates.io dependencies.

## Toolchain

- Rust **stable** (MSRV is effectively "recent stable": edition 2021, no
  nightly features). `rustup component add rustfmt clippy`.
- Optional, only for the live-integration tests: a redis-server and a
  PostgreSQL server (see below). The default test suite needs **no
  services**.
- Linux/macOS. The sandbox/devbox flow below assumes Debian-ish apt.

## The loop

```bash
cargo fmt --all                        # format (CI checks `-- --check`)
cargo clippy --all-targets --all-features -- -D warnings   # lints are errors
cargo test --all-targets                                  # default features
cargo test --all-targets --all-features                   # + sqlite/pg/redis units
cargo build --release --all-features                      # release binary
./target/release/harvest validate --config config.example.json
```

Feature combos are compile-checked in isolation (CI matrix, and worth running
locally after touching any `cfg(feature` block — an all-features run masks
broken gating):

```bash
for f in "" sqlite postgres redis; do
  cargo check --all-targets ${f:+--features $f}
done
```

## Test layout (what lives where, and why)

*(feature-gated extras: `devtools` = the in-crate test harness, see below;
`redis`/`sqlite`/`postgres` = the live-backend integration tests.)*

| Where | Covers |
|---|---|
| `#[cfg(test)]` in `src/*.rs` | the module's unit behavior: config validation & env injection, URL rules, extract/JSONPath, politeness math, ckpt round-trips, redactors, metric shapes — and the engine layer's state machines (job transitions, scheduler modes, breaker clock math, retry curve/budgets, bus ring behavior, supervision policy, accounting bounds) |
| `tests/engine_layer.rs` | the engine layer wired *into the loop*: lifecycle events end-to-end (JSONL), breaker skip counts, bounded-queue backpressure, pop-time deadline expiry, deterministic retry delays, mode equivalence (fifo/priority/fair collect identically), `/runtime` + `/events` over the real admin socket with token auth, drain event pairing with no dangling jobs |
| `tests/e2e.rs` | whole engine against a **loopback mock HTTP server** spun up inside the test (no network, no ports you must manage): crawl → sinks → history → audit → resume |
| `tests/security.rs` | admin auth (401 before 200, constant body), body caps, redirect caps, path-traversal refusal — the Prompt-hardening regressions |
| `tests/cli.rs` | the binary-as-unix-tool contract: SIGPIPE on closed pipes instead of `println!` panic (spawn via `CARGO_BIN_EXE_harvest`) |
| `tests/ops_api.rs` | the whole ops surface wired end-to-end: real `harvest serve` against a loopback origin, admin API auth/correlation/errors, alerts cooldown dedup measured at the webhook, operator audit trail, config redaction + `env:` provenance, health exit codes, bounded 5k-line listing caps |
| `tests/cli_ops.rs` | the ops CLI offline contract: filter/sort/offset semantics, jsonl streaming, deterministic repeat output, and exit-code separation (0 · 1 · 2) with stdout kept data-only |
| `tests/workflow_devtools.rs` | the `harvest::testutil` harness driving a real engine run (cut mid-stream page, retries, events, metrics view) — dev utilities are exercised, not just present |
| `src/testutil.rs` (`devtools` feature / unit builds only) | deterministic clock, temp fixture, mock origin (respond/redirect/cut/garbage/delay), in-memory sink, event collector, metrics recorder — never compiled into normal builds |
| `tests/dist.rs` | queue semantics via `Queue::{push,pop,ack,requeue,dead,sweep_inflight,counts}` — **skips unless `HARVEST_TEST_REDIS=1`** |
| `tests/pg.rs` | real-sink idempotency (same run twice ⇒ no dupes) — **skips unless `HARVEST_TEST_PG=1`** |

Run one file while iterating: `cargo test --test security`, or by name:
`cargo test sink::tests::csv`.

### Live-integration tests, local services

```bash
# Redis (db 9, no persistence, non-default port to avoid clashing with dev redis)
redis-server --port 6399 --save '' --daemonize yes --dir /tmp
export HARVEST_TEST_REDIS=1
export HARVEST_TEST_REDIS_URL=redis://127.0.0.1:6399/9

# Postgres (any local 15+; CI uses postgres:17-alpine)
sudo apt-get install -y postgresql && sudo pg_ctlcluster 17 main start
sudo -u postgres psql -c "CREATE ROLE harvest LOGIN PASSWORD 'harvest'" \
   -c "CREATE DATABASE harvest OWNER harvest"
export HARVEST_TEST_PG=1
export HARVEST_PG_DSN="host=127.0.0.1 port=5432 user=harvest password=harvest dbname=harvest"

cargo test --all-targets --all-features
```

Without those env vars the two files **skip silently by design** (a fresh
clone must be green with zero services); CI's `test-all` job sets them and
its postgres/redis service ports are exactly the defaults above — change one
side, change both.

### Fault routes, property sweeps, and benches

The devtools mock origin offers *deterministic* fault injection:
`Route::flaky(n, code, then)` (first n requests to that route fail with
`code`, then it delegates), `Route::Reset` (connection closed with no
response bytes), and `Route::hang(d)` (delay, then reset). Hit counting is
per-route-slot under the single accept loop, so the flaky ladder is exact
even while the engine runs concurrently — randomness that re-runs identically
is the entire point (`tests/faults.rs`).

`tests/properties.rs` runs seeded-PRNG sweeps over the algebraic invariants:
id parse/reject under deformation, budget accounting exactness under random
hold/release interleaving, chunked exports concatenating byte-identically to
the single-file export (and `verify` PASS at every cap), migration idempotence
and future-stamp fail-closure. A failing seed reproduces by re-running it.

`cargo bench` compiles `benches/throughput.rs` — a zero-dependency ns/op
table for the hot paths (id parsing, budget takes, metrics counters/render,
extract + pipeline apply, export run/verify). Numbers are machine-relative:
record them here on this box, compare only against a previous run on the same
box. Current reference (release, dev container): RecordId::parse 64 ns ·
TargetId::parse 52 ns · budget try_take+release 20 ns · metric counter 6 ns ·
prometheus render 3.2 µs · html extract 378 ns · pipeline apply 391 ns ·
export 5k chunked 26 ms · verify 2.4 ms.

## CI jobs (`.github/workflows/ci.yml`)

1. `fmt` — `cargo fmt --all -- --check`
2. `clippy` — `--all-targets --all-features -- -D warnings`
3. `test-default` — no services
4. `test-all` — full suite **with** live redis:7 + postgres:17 services,
   `HARVEST_TEST_*` set explicitly (a mis-configured service must fail, not
   skip)
5. `features` — `cargo check --all-targets` per feature combo (`""`, sqlite,
   postgres, redis)
6. `release` — release build + `--version`, `validate config.example.json`,
   and an attest round-trip guard: a generated key must **reject** an
   unsigned artifact (exit non-zero).

No secrets live in CI; service passwords are ephemeral CI-scoped `harvest`
roles.

## Repository layout

```
src/main.rs      CLI parsing only (clap); each subcommand is ~10 lines of
                 arg plumbing over the lib. Keep behavior OUT of this file.
src/lib.rs       crate docs + re-exports
src/config.rs    ALL knobs, defaults, env injection, validate(), redactors —
                 the one place startup errors originate
src/engine.rs    the crawl: bounded worker pool over the job queue, robots,
                 pacing hooks, dedup, extraction wiring, ckpt ticks, summary
src/policy.rs    per-domain token bucket + min-interval + block breaker
                 ("compliance-first" enforced here, not in config hopes)
src/robots.rs    RFC-9309-enough robots.txt parser (UA groups, longest
                 match, Crawl-delay)
src/fetcher.rs   one request: UA, timeouts, redirect/body caps, retry with
                 jitter, Blocked/Failed/Permanent classification
src/proxy.rs     pool with health scoring — failover for reliability only
src/extract.rs   CSS/JSON extraction rules → serde values, text fingerprint
src/sink.rs      Sink trait + jsonl/csv/dataset/http/null/sqlite
src/pgsink.rs    postgres sink (off-thread client; same pattern as http)
src/export.rs    client delivery folders: csv+jsonl+manifest+signature
src/ckpt.rs      checkpoint shape, atomic write, resume refusal rules
src/reval.rs     conditional-GET + cached link graph for re-crawls
src/audit.rs     per-host politeness receipt (audit.json + .sig)
src/sign.rs      HMAC-SHA256 detached attestation (key handling)
src/metrics.rs   16 flat counters + Prometheus text
src/admin.rs     admin HTTP/1.1 API + dashboard (hand-rolled — deliberate:
                 no framework to drift; see docs/api.md for the contract)
src/runner.rs    scheduler loop (serve) + append-only run-history log
src/alerts.rs    post-run SLO evaluation + single webhook notify
src/dist.rs      redis queue: push/pop/ack/requeue/dead/counts/sweep
tests/           integration suites (table above)
examples/        shippable configs + real output samples — every config must
                 pass `harvest validate --config …` before you touch it
ops/             docker-compose + prometheus/grafana provisioning
demo/            self-contained mock site (server.py + robots + sitemap)
                 for offline demos and the README quickstart
docs/            architecture / api / configuration / deployment /
                 troubleshooting / development
```

## Contribution workflow

1. Branch off `main`. One concern per PR (a `config.rs` change and an
   `engine.rs` behavior change in one PR costs review days).
2. Behavior change ⇒ update the affected doc **in the same PR** — docs are
   part of the product surface here (this repo ships the promises in
   README/docs as contracts, so stale docs are a bug, not a nit).
3. New failure mode ⇒ new test that pins it (see `tests/security.rs` style:
   assert the *error text pattern*, so message drift is deliberate).
4. New config field ⇒ `config.rs` default + `validate()` rule + docs table
   entry + one validation test. No optional field without a refusal test.
5. `cargo fmt` + clippy clean + full battery green locally, CI green ⇒
   squash-merge. `CHANGELOG.md` gets an entry under `[Unreleased]`.

## Style notes that recur in review

- Errors: `anyhow::Context` chains that name the *file and remedy*
  (`attest key not found at X (generate one with harvest attest --gen-key …)`)
  — never print secrets or full proxied URLs (use
  `config::redact_credentials`).
- `#[cfg(feature = …)]` guards exist so the default build has zero optional
  deps; keep new heavy deps behind a feature.
- Poison-tolerant mutex pattern: `.lock().unwrap_or_else(|p| p.into_inner())`
  — a panicking task must not brick long runs; if you introduce a new
  cross-task lock, follow it and add a test that the run survives.
- Tests spawn their own loopback mock servers (`0.0.0.0:0` + read back the
  port); never hardcode ports in tests.

Consequence records for the choices above live in `docs/adr/`; the full
gate matrix (and how each suite is split) is `docs/testing.md`.
