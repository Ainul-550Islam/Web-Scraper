# Deployment

Three supported topologies; all use the same binary and config format.
Development vs production differences are marked. Secrets are never baked
into images or configs — see `docs/configuration.md` for `${VAR}` injection.

## 1 · Local / single box (development)

```bash
cargo build --release --all-features        # or grab a prebuilt binary
./target/release/harvest validate --config config.example.json
./target/release/harvest run    --config config.example.json
# scheduler mode (dev): runs every schedule.every_secs until Ctrl-C
./target/release/harvest serve  --config config.example.json
```

The default `config.example.json` binds admin to `127.0.0.1:9678` with no
token — correct for a laptop, **not** for a host with a routable interface.
Dashboard: `http://127.0.0.1:9678/`. Stop with Ctrl-C — in-flight pages
finish, sinks flush, history/audit land, then exit 0.

Long crawls (production): keep `ckpt_path` set; a crashed or restarted box
continues with `harvest run --config … --resume` (refused if the policy
changed in between — edit config and start fresh in that case).

## 2 · Docker (single container)

```bash
docker build -t harvest-engine:latest .
docker run -d --name harvest \
  -v "$PWD/myconfig":/etc/harvest:ro \
  -v harvest-data:/var/lib/harvest \
  -p 127.0.0.1:9678:9678 \
  --env-file harvest.env \
  harvest-engine:latest
```

What the image does for you (`Dockerfile`):

- multi-stage rustls build (`--features "sqlite postgres"` baked in; add
  `redis` by rebuilding with it) — **no secrets, no config in the image**;
- runtime: `debian:bookworm-slim` + CA certs only; **non-root uid 10001**;
- `/var/lib/harvest` pre-created and chowned to the service user (the only
  writable path — point file-sink `path`s and history/audit/ckpt under it);
- `HEALTHCHECK` hits `/health` over `/dev/tcp` (auth-exempt by design, no
  curl in the image); assumes `metrics_addr` on 9678 — adjust the probe if
  you move it;
- `ENTRYPOINT ["harvest"]`, default `CMD run --config /etc/harvest/config.json`.

Required for a routable bind: `HARVEST_ADMIN_TOKEN` in `harvest.env` plus
`"admin_token": "${HARVEST_ADMIN_TOKEN}"` in the config; without them the
container exits 1 at startup with the remedy in the message — intentional.

Note on stop semantics: the engine drains gracefully on **SIGINT**; docker
stop sends SIGTERM (immediate kill) by default, so for drain-on-stop use
`docker run --stop-signal SIGINT …` (compose: `stop_signal: SIGINT`,
already set in `ops/docker-compose.yml`).

## 3 · Docker Compose (engine + Prometheus + Grafana)

```bash
export GRAFANA_ADMIN_PASSWORD="$(head -c18 /dev/urandom | base64)"
export HARVEST_ADMIN_TOKEN="$(head -c24 /dev/urandom | base64)"
docker compose -f ops/docker-compose.yml up -d
```

Topology: `harvest :9678 (127.0.0.1)` ⇐ scrape ⇒ `prometheus :9090
(127.0.0.1)` ⇒ datasource ⇒ `grafana :3000 (127.0.0.1)` with provisioned
dashboard `ops/grafana/dashboards/harvest.json`.

Hardened defaults already in the file — keep them for anything off-laptop:
all host ports bound to `127.0.0.1`; harvest container `read_only: true` +
`tmpfs /tmp` + `cap_drop: [ALL]` + `no-new-privileges`; config mounted `:ro`;
Grafana sign-up off; the compose Grafana password is a **dev placeholder**
overridden by `GRAFANA_ADMIN_PASSWORD`. Prometheus sends the bearer token
from `$HARVEST_ADMIN_TOKEN` so scrape keeps working once you enable auth
(that env is passed to both containers — same single secret).

Real exposure answer: put a TLS reverse proxy (nginx/traefik/Caddy) in
front of Grafana/Prometheus rather than re-binding ports; Prometheus itself
has no auth story.

## Databases

**PostgreSQL** — engine auto-creates `harvest_records` (idempotent inserts
keyed by `(run, target, fingerprint)`). You provide role/db (engine does not
create those):

```sql
CREATE ROLE harvest LOGIN PASSWORD 'from-secret-manager';
CREATE DATABASE harvest OWNER harvest;
```

Config: `"output": {"kind":"postgres","dsn":"host=db port=5432 user=harvest
password=${PG_PASSWORD} dbname=harvest sslmode=require"}` (dev: drop
`sslmode`). Backups = plain `pg_dump harvest`; harvest never expires rows,
retention is your DBA's call.

**Redis** (distributed mode only) — no schema; harvest uses keyed lists/sets
under one prefix (`{prefix}:jobs`, `{prefix}:inflight`, `{prefix}:dlq`,
`{prefix}:seen_urls`, `{prefix}:seen_fp`, and a shared robots cache at
`{prefix}:robots:{scheme}://{authority}`), so a shared instance is safe if
you keep a unique prefix per crawl. Use `rediss://` for anything not on
localhost (enforced when AUTH + remote host). No persistence needed for
correctness (queue loss = re-push). A job a killed worker had checked out
stays in `{prefix}:inflight` until swept back — `harvest queue` prints
jobs/inflight/dlq/dedup counts so strays are visible; the sweep itself is the
`Queue::sweep_inflight` library method (run it only with no live workers on
that prefix, or live jobs get double-processed).

Distributed flow (`--features redis` binary on every node):

```bash
harvest push   --config prod.json --reset         # seed queue once
harvest worker --config prod.json &               # N shards anywhere
harvest queue  --config prod.json                 # depth snapshot (jobs/inflight/dlq/dedup sets)
# workers exit on their own once the queue stays empty for --idle-secs (5s default);
# each writes out/…jsonl.p<pid>; merge with
# `harvest export --from a.jsonl --from b.jsonl`.
```

While a shard works a job it heartbeats a **lease** (TTL 30 s, refreshed at
TTL/3): key `<prefix>:lease:<sha>`, value `<worker>-s<seq>-<hex>` naming the
holder. Startup strays (`inflight` entries no worker owns) are swept back to
`jobs` — `harvest push --reset` includes that sweep — and the sweep *skips
anything with a fresh lease*, so restarting the cohort while one slow shard is
still crawling cannot double-process its job. `--name` sets the worker label
(an id slug; default `w-<pid>`); `--prefix` must be a redis key token
(`a-z A-Z 0-9 _ - :`), because keys are built by concatenation. Transient redis
loss is backed off and retried (`queue_errors` in the stats line), not fatal.

## Health, logs, metrics

- Liveness: `GET /health` (no auth, constant body) — wired as Docker
  HEALTHCHECK and usable as a K8s probe.
- Readiness semantics: the admin server binds at start; `/stats.running`
  reflects whether a crawl loop is active right now.
- Logs: stdout only, JSON-ish `tracing` lines, tuned via `RUST_LOG`
  (`harvest=trace` for request-level detail). No file appenders (let the
  container runtime/ journald own rotation). Credential-bearing URLs are
  redacted at the source.
- Metrics: `/metrics` Prometheus text; counters only (names in
  `docs/api.md`); alert webhooks push per-run SLO failures instead of
  requiring a scraper.

## Delivery: chunked exports, resume, verification

Large deliveries are chunked at record boundaries and finalized atomically —
payload parts are written under `.staging/`, moved into place, and only then
does `manifest.json` appear: **a directory with a manifest is complete by
construction**. If an export dies mid-flight (no manifest), re-running with
`--resume` validates every part on disk by sha256 (plus an input-file
fingerprint) before reusing it; anything stale or missing is regenerated, and
completed parts are never rewritten.

```bash
harvest export --from out/run.jsonl --to deliver/ --max-part-bytes 268435456 \
               --key ops/harvest.key        # 256MiB self-describing parts, signed manifest
harvest export --from out/run.jsonl --to deliver/ --max-part-bytes 268435456 \
               --key ops/harvest.key --resume   # after a crash; keeps valid parts
harvest verify --dir deliver/ --key ops/harvest.key   # exit 0 only when every
                                                       # part + the manifest signature match
```

`verify` checks presence, bytes, sha256, line counts, the record total, and —
with a key — the detached manifest signature; unexpected files are reported as
`extras` (informational). Consumers of the folder should treat `verify` PASS as
the acceptance gate; the folder is safe to hand over file-by-file (parts are
independent).

## Filesystem & permissions

Single writable directory for everything (files, dataset, sqlite,
history, audit, checkpoints): make it owned by the runtime uid
(`chown 10001`, or your service user) and mount/volume it — the engine
creates parents (`out/…`) but cannot chown what it does not own. Attest
key file: keep 0600 (engine refuses to *write* keys except via
`attest --gen-key`, which sets 0600 and won't overwrite).
`audit.json.sig` / `manifest.json.sig` travel with their artifacts —
signature files are part of a delivery, copy them together.

## Backups worth taking

`schedule.history_path` (run ledger), `audit.json(+.sig)` per delivery,
`revalidate_store` (losing it = one slower re-crawl, nothing wrong),
`dlq_path` (unfinished work), attest key (loss = old receipts unverifiable —
archive read-only with the deliveries), plus your Postgres dumps.
Checkpoint files are transient (deleted on completion) — never a backup
target.
