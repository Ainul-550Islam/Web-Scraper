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
Dashboard: `http://127.0.0.1:9678/`. Stop with Ctrl-C — active requests
cancel, owned sink workers drain/join, history/audit land, then exit 0.

Long crawls (production): keep `ckpt_path` set; a crashed or restarted box
continues with `harvest run --config … --resume` (refused if the policy
changed in between — edit config and start fresh in that case).

## 2 · Docker (single container)

```bash
docker build -t harvest-engine:latest .
docker run -d --name harvest \
  -v "$PWD/myconfig.json":/etc/harvest/config.json:ro \
  -v harvest-data:/var/lib/harvest \
  -p 127.0.0.1:9678:9678 \
  --env-file harvest.env \
  harvest-engine:latest serve --config /etc/harvest/config.json
```

For a published port, set `metrics_addr` to `0.0.0.0:9678` inside the
container and set `"admin_token": "${HARVEST_ADMIN_TOKEN}"`; keep the host
side of `-p` on `127.0.0.1` unless a TLS/authenticated front end is in place.
Put every file-sink, audit, checkpoint, revalidation, and history path under
`/var/lib/harvest`, the only writable data volume in the read-only deployment.

What the image does for you (`Dockerfile`):

- multi-stage rustls build (`--features "sqlite postgres redis"` baked in; change
  `--build-arg HARVEST_FEATURES="…"` if needed) — **no secrets, no config in the image**;
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

The engine drains on **SIGINT and SIGTERM**. Compose uses SIGTERM and a
45-second stop grace. Active requests and pacing/storage waits cancel;
HTTP/Postgres sink workers use an aggregate 15-second drain deadline and
are joined before finalization. Filesystem/kernel stalls are not something
an application-level timeout can reliably interrupt.

## 3 · Docker Compose (engine + Prometheus + Grafana)

```bash
cp -n ops/harvest/config.example.json ops/harvest/config.json
# Replace the loopback demo target with sources you are authorized to crawl.
export GRAFANA_ADMIN_PASSWORD="$(head -c18 /dev/urandom | base64)"
export HARVEST_ADMIN_TOKEN="$(head -c24 /dev/urandom | base64)"
docker compose -f ops/docker-compose.yml up --build -d
```

The Compose starter config is deliberate: the engine listens on
`0.0.0.0:9678` **inside the Compose network** so the console, Prometheus, and
host-published port can reach it; the bearer token is mandatory, while the
published host port is still bound only to `127.0.0.1`. Its writable output,
audit, checkpoint, revalidation, and history files live under
`/var/lib/harvest`. Replace the example target and user-agent contact before
starting a real crawl.

Topology: console `:8088`, engine `:9678`, Prometheus `:9090`, and Grafana
`:3000` are all published on host loopback. Inside Compose, console and
Prometheus reach the `harvest:9678` service; Grafana uses Prometheus as its
datasource. Dashboard: `ops/grafana/dashboards/harvest.json`.

Hardened defaults already in the file — keep them for anything off-laptop:
all host ports bound to `127.0.0.1`; harvest container `read_only: true` +
`tmpfs /tmp` + `cap_drop: [ALL]` + `no-new-privileges`; config mounted `:ro`;
Grafana sign-up off; a unique `GRAFANA_ADMIN_PASSWORD` is mandatory and is
not hard-coded. `HARVEST_ADMIN_TOKEN` is required by both the engine and
Prometheus. Prometheus does **not** expand arbitrary YAML credential
strings. Its entrypoint writes the token to a mode-0600 runtime file, unsets
the environment variable, and YAML uses `authorization.credentials_file`.
The console uses an unprivileged nginx image (uid/gid 101) and a writable
`/tmp` for pid/temporary files; production Host **including its port** is
preserved with `$http_host`, so authenticated browser mutations remain
same-origin after reverse proxying.

The engine drains on both SIGINT and SIGTERM; Compose sends SIGTERM with a
45-second grace period. For public exposure, put a TLS reverse proxy
(nginx/traefik/Caddy) in front of the services rather than re-binding host
ports; Prometheus itself has no auth story.

## Databases

**PostgreSQL** — engine auto-creates `harvest_records` (idempotent inserts
keyed by `(run, target, fingerprint)`). You provide role/db (engine does not
create those):

```sql
CREATE ROLE harvest LOGIN PASSWORD 'from-secret-manager';
CREATE DATABASE harvest OWNER harvest;
```

Config: `"output": {"kind":"postgres","dsn":"host=db port=5432 user=harvest
password=… dbname=harvest sslmode=require"}`. Inject the **whole DSN** as
`"dsn":"${HARVEST_PG_DSN}"`; embedded `${VAR}` fragments are deliberately
not expanded. Remote connections require `sslmode=require` and platform CA
verification (no hostname/certificate bypass); loopback may use
`sslmode=disable` for development. Backups = plain `pg_dump harvest`; harvest never expires rows,
retention is your DBA's call.

**Redis** (distributed mode only) — no schema; harvest uses keyed lists/sets
under one prefix (`{prefix}:jobs`, `{prefix}:inflight`, `{prefix}:dlq`,
`{prefix}:seen_urls`, `{prefix}:seen_fp`, and a shared robots cache at
`{prefix}:robots:{scheme}://{authority}`), so a shared instance is safe if
you keep a unique prefix per crawl. Use `rediss://` for anything not on
localhost (enforced for every remote host, with or without AUTH). No persistence needed for
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


## Service and Kubernetes recipes

`deploy/systemd/harvest.service.example` runs the **engine**, not a static
file server. Install nginx/Caddy separately for `web/dist` and proxy `/api`
from the same origin. Put whole-value environment secrets in a root-owned
`/etc/harvest/harvest.env` (mode 0600); `HARVEST_ADMIN_TOKEN_FILE` is not an
implemented engine option. The console Kubernetes deployment now declares
uid/gid 101, drops capabilities and provides a bounded writable tmpfs.
It assumes an existing `harvest-engine` service; it is not a full Kubernetes
engine/Redis/Postgres installation.

For resource limits, stricter redirect/robots behavior, source formats,
private snapshots and recovery caveats, see [review hardening](review-hardening.md).
