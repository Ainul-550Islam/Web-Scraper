# Buyer demo script — every claim below is a command you can run

This is a guided evaluation of Harvest: one terminal, one browser, zero
external services. Every section states what to run and what you should see.
Nothing here cites metrics the repo cannot produce on your machine — where a
number matters, the command that prints it is given instead.

Prereq: Linux/macOS, Rust stable (1.75+), Node 20+, Python 3. Paths are
relative to the repo root unless stated. `out/`, `web/dist/`,
`web/node_modules/` are generated and gitignored.

---

## A. What you are buying (one paragraph)

A single Rust binary (`harvest`) that crawls/fetches configured targets with
explicit politeness (robots.txt, per-domain rate limits, budgets), extracts
typed fields (JSONPath/CSS), validates against per-target schemas, quarantines
instead of silently dropping bad records, writes JSONL (or SQLite with the
`sqlite` feature, Postgres with `pg`), checkpoints for resume, signs audit
and export manifests with HMAC attestation, and serves a read-mostly admin
API + React console for inspection. It is not a hosted product: you run it.

## B. Build

```sh
cargo build --release                      # engine + CLI
python3 -c "import http.server"            # demo mock site needs python3 only
```
Expected: `Finished release profile`. Toolchain volatility is your machine's,
not the repo's.

## C. The 60-second demo

```sh
python3 demo/server.py 8099 &              # deterministic mock website
cargo run --release -- run --config examples/config.basic.json
kill %1
```
`examples/config.basic.json` points its single target at
`http://127.0.0.1:8099`, writes to `out/basic.jsonl`, and serves the admin
API on `127.0.0.1:9678` for the run's lifetime. One-shot `run` exits 0.

## D. Read the output yourself

```sh
head -1 out/basic.jsonl
```
Each line is JSON: `{run, target, url, depth, ts, fingerprint, egress,
fields:{...}}` — provenance fields first-class, extracted values under
`fields`. `ts` is unix seconds of the sink write.

## E. Scheduler mode

```sh
python3 demo/server.py 8099 &
cargo run --release -- serve --config examples/config.basic.json --runs 2
kill %1
```
`serve` repeats the job on `schedule.every_secs`; `--runs 2` caps it so the
demo terminates. SIGINT between cycles drains gracefully (exit 0).

## F. The console (web) against your engine

Keep `serve` running (no `--runs`, then stop it later with Ctrl-C), then in a
second terminal:

```sh
cd web && npm ci && npm run dev
```
Open the printed localhost URL. The dev server proxies `/api` to the engine
(see `web/vite.config.ts`); the header shows the connected backend. With
`VITE_MOCK=1 npm run dev` you get the same UI on canned data — the banner
says DEMO MODE loudly; mocks are compiled out of production builds entirely,
so a demo render can never masquerade as live data.

## G. Health and liveness

```sh
curl -s localhost:9678/health | python3 -m json.tool   # {status, running}
curl -s -o /dev/null -w "%{http_code}\n" localhost:9678/live
```
Both routes are open by design (no token required, loopback or not). Health
is `{status, running}` — it deliberately exposes no version/secret fields.

## H. Prometheus metrics

```sh
curl -s localhost:9678/metrics | head -20
```
Text exposition: counters per request/record/budget/breaker family, e.g.
`harvest_records_written_total`, `harvest_pages_fetched_total`. These are the
same counters the console reads (`GET /stats`, `/runtime`).

## I. Jobs and runs

```sh
curl -s localhost:9678/jobs   | python3 -m json.tool | head
curl -s "localhost:9678/runs?limit=5&sort=started_at&order=desc" | python3 -m json.tool | head
```
`/jobs` = current queue/ledger view; `/runs` = persisted run history
(`out/runs.jsonl`, tail-limited window 512 — see docs/api). The console's
Runs page draws its timeline from these same fields (no invented timestamps).

## J. Workers

```sh
curl -s localhost:9678/workers | python3 -m json.tool
```
Live supervisor snapshot: registered/busy counts and per-worker state. On a
Redis build (`--features redis`) workers can run as separate processes;
the same endpoint aggregates them.

## K. Queue

```sh
curl -s localhost:9678/queue | python3 -m json.tool
```
Depth, inflight, per-domain fairness state. Empty-but-healthy looks like
`depth: 0` while idle — the UI says so instead of faking activity.

## L. Proxy pool

```sh
curl -s localhost:9678/proxies | python3 -m json.tool
```
Each egress: bucket (healthy/degraded/down), cooldown, in-use. The demo
config uses `direct` egress; add proxies under `proxies: []` to watch
failover mechanics (docs/api covers the fields).

## M. Quality and quarantine

```sh
curl -s localhost:9678/stats | python3 -m json.tool | grep -E "quarantined|deduped|drift|written"
curl -s localhost:9678/pipeline | python3 -m json.tool | head -30
```
`records_quarantined` > 0 means bad rows were diverted to
`out/quarantine.basic.jsonl` (per config), **never dropped silently**;
`schema_drifts` counts contract deviations; `/pipeline` is the sequential
funnel the Quality page renders with quarantine visibly separate.

## N. Storage lanes

```sh
curl -s localhost:9678/storage | python3 -m json.tool
```
Per lane (sink/checkpoint/quarantine/history): configured path, exists?,
bytes. The console's Storage page and Overview's STORAGE group render exactly
this — no lane is shown as healthy that the engine did not report.

## O. Alerts — one of only two mutations

```sh
curl -s localhost:9678/alerts | python3 -m json.tool | head
curl -s -X POST "localhost:9678/alerts/ack?event=<id>"   # ack an alert
```
Alert rules fire from config thresholds; delivery is logged (`notified`).
`/alerts/ack` and `/ops/note` are the **only** POST routes; everything else
405s with a message that says so.

## P. Ops notes — the other mutation

```sh
curl -s -X POST localhost:9678/ops/note -d '{"text":"demo pass 1 complete"}'
curl -s localhost:9678/ops | python3 -m json.tool | head
```
Notes land in the ops log consumed by the console Ops/Audit views.

## Q. Audit trail

```sh
curl -s "localhost:9678/audit?limit=5" | python3 -m json.tool | head
```
Every stateful decision (fetch, robots denial, breaker open, sink write,
export) is a hash-chained line when `audit_path` is set. The demo config
leaves it off to show the honest "audit lane off" state; flip
`"audit_path": "out/audit.json"` in a copy of the config to see rows.

## R. Event tail

```sh
curl -s "localhost:9678/events?limit=20" | python3 -m json.tool | head
```
Structured runtime events (`kind`, `seq`, `detail`) — what the console's
Activity view and Overview sparkline replay. `?since_seq=` gives incremental
tails.

## S. Config as code, validated before first byte

```sh
cargo run --release -- validate --config examples/config.basic.json
jq '.max_concurrency = 99999' examples/config.basic.json > /tmp/bad.json
cargo run --release -- validate --config /tmp/bad.json; echo "exit:$?"
```
Expected: first exits 0 with a summary; the second fails with a named
constraint (validation is enforced at startup — no partially-bad run).

## T. Target Builder

In the console: Configuration Studio → Target Builder → "Add target". Each
step (Identity, Crawl, Extraction, Schema contract) edits the draft; the
drawer's live "Generated target config" card shows the exact JSON the engine
would read. Drafts export via "Export draft" → feed it back to
`harvest validate`. The admin API is read-only for config by design.

## U. Export a delivery and verify it

```sh
cargo run --release -- export --from out/basic.jsonl --to /tmp/deliver
cargo run --release -- verify --dir /tmp/deliver
ls /tmp/deliver
```
Export writes combined `records.jsonl` + `records.csv` + `manifest.json`
(per-file sha256, byte counts, line/record totals; `--split_target` gives
per-target pairs; `--max-part-bytes` chunks at record boundaries for big
deliveries; `--resume` keeps parts whose sha256 still matches). `verify`
re-checks everything and exits 1 on any mismatch.

## V. Attest the manifest

```sh
cargo run --release -- attest --gen-key /tmp/attest.key
cargo run --release -- export --from out/basic.jsonl --to /tmp/deliver --key /tmp/attest.key
cargo run --release -- verify --dir /tmp/deliver --key /tmp/attest.key
cargo run --release -- attest --verify /tmp/deliver/manifest.json --key /tmp/attest.key
```
HMAC-SHA256 detached signature (`manifest.json.sig`); mode 0600 on key
creation. Verifying a file that was never signed must fail loudly — CI's own
smoke asserts exactly that (see .github/workflows/ci.yml).

## W. Checkpoint and resume

`ckpt_path` is set in the demo config. Kill `serve` mid-cycle, then:

```sh
cargo run --release -- run --config examples/config.basic.json --resume
```
Resume replays from the checkpoint and refuses if the config (policy sha)
changed since it was written. A completed run deletes the checkpoint —
re-running without `--resume` simply starts fresh.

## X. Robots and politeness, shown not promised

```sh
grep -n "respect_robots\|per_domain\|delay\|budget" examples/config.basic.json
```
`respect_robots: true` fetches and obeys robots.txt (cached; denials counted:
`robots_denials`); per-domain polite delay between requests; page/record/
time budgets are enforced engine-wide (`budget_denied_*` counters). The demo
mock site ships a permissive robots.txt — deny one path there to watch the
engine skip it and count the denial.

## Y. Security model

Loopback-first: any route except `/health` + `/live` requires a bearer token
once `admin_token` is configured (`Authorization: Bearer …`); the API echoes
config with secrets redacted (`admin_token: null` in `/config`). Tokens live
in memory in the console (never localStorage — ADR 0006). No SQL endpoint, no
shell endpoint, config POSTs 405. Full model: [SECURITY.md](../SECURITY.md).

## Z. DEMO vs LIVE

The console has two modes: LIVE (default) talks to your admin API only;
DEMO MODE (`VITE_MOCK=1` in the dev server) renders fixtures and
shows a permanent banner. A live console never invents numbers: unanswered
endpoints render "—"/offline states, and the API Explorer lists only routes
that exist (contract-tested against the router).

## AA. Performance — measure, don't trust

```sh
cargo bench --bench throughput                     # engine micro-corpus
python3 - <<'EOF'
import json,os;print(os.path.getsize("out/basic.jsonl"),"bytes sink output")
EOF
ls -l web/dist/assets | awk '{s+=$5} END {print s,"bytes console dist (pre-gzip)"}'
```
Docs deliberately do not print benchmark tables: run the commands on your
hardware. Bundle size note: see README ("Size, measured").

## AB. Troubleshooting

- Console says "awaiting admin API": engine down or wrong port (vite proxy →
  127.0.0.1:9678; check `metrics_addr` in the config).
- `address already in use`: previous demo still bound 8099/9678/4321 — stop it.
- Export verify exit 1: the dir was touched after manifest creation (expected).
- Redis/Postgres features need their servers + `--features redis`/`pg` builds.

## AC. Acceptance checklist (tick as you run)

- [ ] C–E run to completion, exit 0, sink file exists (D)
- [ ] F console boots against the live engine and shows the same numbers H/I give
- [ ] M quarantine visible and separate in Quality view
- [ ] S invalid config refused before any fetch
- [ ] U/V delivery verifiable + attestable, unsigned verify fails
- [ ] Y `/config` shows `admin_token: null` over the API
- [ ] Z demo banner present in demo mode, absent in live
- [ ] `make all` (fmt, clippy-free lints, tests, build, links, smoke) green
- [ ] `cd web && npm run e2e:buyer` → `BUYER E2E: 18 passed, 0 failed`
