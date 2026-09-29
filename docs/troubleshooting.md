# Troubleshooting

Each entry: symptom → likely cause → how to confirm → fix. Quoted strings are
the engine's actual output. Where it says "log line", read stdout of the
harvest process (journald / `docker logs harvest`) — logs go to stdout only.

## Startup refuses the config

**`config_version 3 is newer than this build understands (max 2) — upgrade
harvest before loading this config, do not downgrade the file`** — the config
was stamped by a *newer* binary. Downgrading configs is refused by design;
upgrade harvest. `harvest config migrate` on a box with the older binary shows
the accepted range without writing anything (`--dry-run`).

**`unknown config key `cuncurrancy` — did you mean `concurrency`?`** — the
load-time typo gate. Fix the key. Genuinely custom metadata must live under a
`_`-prefixed key (`_meta: {...}` is ignored as a comment, like the examples'
`_note`).

**`error: config validation failed: <rule>: <detail>`**
Any of the checks in `docs/configuration.md` §"Validation failures" — the
message names the exact rule. Confirm: `harvest validate --config f.json`
prints *all* violations joined by `; `, not just the first. Fix the config;
do not work around it — every rule exists to prevent a specific production
accident (open admin, unbounded timeouts, feature-less sinks).

**`error: unknown configuration format — pass a .json file`** — you passed
TOML/YAML. Convert; the format on disk is JSON only.

**`environment variable NAME is not set (referenced in config value
"${NAME}")`** — the config expects env injection that didn't happen.
Confirm: `env | grep NAME` in the *process's* environment (for compose:
inside the container: `docker compose exec harvest env | grep NAME` — the
host shell having it is not enough; it must be exported to the service via
`environment:`/`--env-file`). Fix by exporting/injecting, or remove the
`"${NAME}"` reference. Do not paste the secret into the config file.

**`postgres sink requested but binary lacks `--features postgres``** (same
shape for sqlite) — the binary was built without the feature the config
selects. `harvest validate` is the cheapest probe; it's exactly this check.
Fix: rebuild (`cargo build --release --features "postgres"` etc.) or switch
the sink kind. The Docker image ships sqlite+postgres built in. Same deal
for distributed mode: `push`/`worker`/`queue` subcommands exist only when
the binary was built `--features redis` — if they're missing from
`harvest --help`, that's the cause; no network debugging needed.

## The crawl ends immediately / does nothing

- **Everything 403/429**: you arrived under the configured UA and the site
  refuses it. Confirm: `harvest audit --file out/…/audit.json` shows
  `blocked` + `robots.denials` for the host; and `GET /metrics`
  `harvest_requests_blocked`. Fix: adjust `user_agent` (contact address!),
  slow `domain_policies` down, or ask the site for permission — do **not**
  set `respect_robots:false` unless you have it, and never set a browser UA
  (validate refuses it while robots enforcement is on).
- **`denied by robots.txt` debug lines then 0 pages**: robots.txt genuinely
  forbids your UA at the start URL (visible with `RUST_LOG=harvest=debug`).
  That is the product working. Confirm with
  `curl -A "<your UA>" https://site/robots.txt`.
- **Records written but full of `null`s**: selectors matched nothing —
  non-matching fields extract to `null` and the record is still written (so
  you can audit what the extractor saw). Confirm: `grep -c '"title":null'
  out/records.jsonl`. Fix: run `harvest fetch <url>` to get the live body and
  re-check the CSS against it; pages that moved behind JS rendering will
  never match — this engine parses served HTML only.

## One host keeps failing

**dlq line `"reason":"blocked:429"` / `"blocked:403"`** — refusal; the engine
backed off and eventually abandoned the host for the run
(`block_breaker_count`). Confirm: `/metrics` `harvest_requests_blocked`,
`harvest_domains_abandoned`. Fix: lower `requests_per_sec`, raise
`block_backoff_secs`; replay later with `run --retry-dlq` — successful URLs
are dropped from the file; a failing entry's `attempts` increments on each
rewrite and it is silently dropped after `dlq_max_attempts` ("dead for
good" — stop retrying programmatically and investigate, or edit the file
deliberately if you know better).

**`request failed after retries` with `declared body size N bytes exceeds
max_response_bytes M`** (or `streamed body exceeds …`) — page (or a redirect
hop) is larger than `max_response_bytes`. Short-circuited, never retried —
retrying a deterministic refusal wastes politeness budget. Confirm with
`curl -sI <url>`. Fix: raise the cap (still bounded!) or exclude those URL
families with `exclude`.

**`request failed after retries` warn lines for every page of a host** —
network path problem, not page slowness (the retry loop already spent
`max_retries` × backoff before printing this). Confirm reachability/egress from the same box:
`harvest fetch <url>` (uses the engine's stack, no robots). For proxies:
compare `GET /stats` per-proxy `success_ewma` — the failing proxy shows
first. Fix DNS/routes/proxy list, not `request_timeout_secs` (raising the
global timeout slows every genuine hang later).

## Postgres / SQLite / sink writes

**`job failed` warn lines repeating with a driver error** (e.g. rusqlite's
`unable to open database file`) — per-record writes fail while the crawl
continues, so you'll see them *per page*: the cause is connectivity or
filesystem, not the data. For sqlite that message means the file's
**directory** doesn't exist or isn't writable by the runtime uid
(`deployment.md` §Filesystem). For postgres from a container, a DSN saying
`localhost` instead of the service name (`host=db`) is the classic.
Confirm by connecting with the same DSN via `psql`/`sqlite3`.
**Run ends `error: sink POST <url> returned 401` (or `… failed: <io>`)
with exit 1** — batched http sink couldn't deliver the tail; batches already
sent are gone (at-least-once is per attempt, the sink has no local spool).
Fix auth/reachability and re-run the same config — fingerprint dedup makes
re-writes no-ops for sqlite/postgres targets.
**`worker task died: <msg>` at run end** — a worker task panicked (engine
locks are poison-tolerant, so one bad job can't take the run down, but a
genuine bug ends the run with this error instead of a hang). Keep
`ckpt_path` set, capture the message + `RUST_LOG=harvest=debug` rerun, and
report it — that line is the bug's front door.

## Redis (distributed mode)

**`worker requires redis_url (see docs/deployment.md)`** — binary lacks the
`redis` feature: rebuild with `--features redis`.
**`redis <redacted-origin>: Connection refused`** — wrong `--redis`/url or
service down: confirm `redis-cli -u <url> ping`.
**Queue drains but workers idle forever**: another prefix is holding the
work (`harvest queue --prefix X` per candidate prefix). Prefixes are the
isolation boundary.
**`inflight` grows, no progress**: a shard died mid-job. Its lease heartbeat
stopped, so after `LEASE_TTL` (30 s) the entry is reclaimable: run the sweep
(`Queue::sweep_inflight`; `push --reset` sweeps and restarts the queue
wholesale if redoing the crawl is acceptable) and restart the cohort. To see
who owns a stuck entry *before* its lease expires: `redis-cli GET
'<prefix>:lease:*'` prints the holder `<worker>-s<seq>-<hex>`; the worker's
own logs carry the same name (`--name`).
**Per-host pacing feels doubled**: you pointed two crawls at the same Redis
prefix expecting isolation — each crawl must own a prefix; robots/pacing
state is shared per prefix by design, dedup too.

## Admin / monitoring

**`401 Unauthorized` from `/stats` etc.** — expected once `admin_token` is
set. Confirm the token the *server* has (env injection may not have
happened — see the env error above). Fix the caller:
`-H "Authorization: Bearer $HARVEST_ADMIN_TOKEN"`; for Prometheus, the
scrape job needs `authorization.credentials: $HARVEST_ADMIN_TOKEN`
(compose already wires it).
**Port already in use** (`metrics_addr` bind) — two engines share a box.
Second one needs another port: change `metrics_addr` or run `serve`
exclusively. `GET /health` failing while the container is "up": the bind
moved but the Dockerfile HEALTHCHECK assumes 127.0.0.1:9678 — update the
probe with it.
**Grafana panels empty**: Prometheus scrape target down — `curl
http://127.0.0.1:9090/api/v1/targets | jq '.data.activeTargets[] |
{health: .health, lastError: .lastError}'` — a 401 there is the missing
bearer passthrough; connection-refused means you moved `metrics_addr`.
**`/recent` shows `"status":"failed"`** — the history entry carries only
the summary; the run's actual error line is in the process stdout around
that `ts` (unix seconds) — for containers:
`docker logs --since <ts-120s> harvest | grep -i error`.

## Evidence / exports

**`harvest verify` says FAIL on one file** — the part's bytes no longer match
the manifest (mid-copy truncation, editor "fixes", or tampering). Re-copy that
part; if the folder was produced in place, re-run the export with `--resume`
— valid parts are kept, the broken one regenerated. A *missing* manifest is
not corruption: the export never finalized (see deployment docs); re-run
without expecting harm — staging + progress sidecar make it resumable.

**`--resume` did nothing obvious** — resume skips re-*writing* validated
parts, not re-rendering; identical output is the point. It also refuses to
reuse anything when the input files' sha256s or options changed (fingerprint
mismatch ⇒ full rewrite, silent and correct).

**`HMAC verification FAILED: the file changed after it was signed, or the
signature was not produced with this key`** (exit 1, message names the file) —
transit corruption or the wrong key. Confirm the key is the counterpart of
what produced the `.sig` (one key per delivery channel; keys are 0600 hex
files). Fix: re-download/re-copy from source; if the *source* is the problem,
treat the delivery as suspect and re-export.
**`attest key not found at … (generate one with harvest attest --gen-key …)`**
— config names `attest_key_path` but the file isn't in the container/host
path. Mount it read-only; do not regenerate (old receipts die with it).
**`refusing to overwrite existing key at …`** — `--gen-key` against an
existing key; that's the guard working. Keep the existing file.
**`checkpoint belongs to run \`a\`, this config is \`b\` — refusing to
merge`** or **`config changed since the checkpoint was written (policy sha
x vs y) …`** — resume refused on purpose: the crawl would violate rules its
target may have agreed to. Start fresh (delete ckpt_path) or revert the
config. Never hand-edit a checkpoint.
**manifest `skipped_lines > 0`** — non-conforming input lines in your
records JSONL (hand-edited files). Fix at the source; exports deliberately
skip, never invent records.

## Docker

**`Permission denied (os error 13)` writing under `/var/lib/harvest`** — the
volume was created with root ownership before the image's chown applied
(named volumes inherit existing ownership). Fix once, addressing the actual
compose volume:

```bash
V=$(docker volume ls --format '{{.Name}}' | grep harvest-data)
docker run --rm -v "$V:/data" alpine chown 10001 /data
```

or bind-mount a host dir you `chown 10001`.
**Container killed at `docker stop` mid-write** — expected: SIGTERM isn't
drained (SIGINT is). Compose already sets `stop_signal: SIGINT`; for raw
`docker run`, add `--stop-signal SIGINT`. Worst case (hard kill): sinks are
line/txn atomic and the ckpt is whole-file — resume sees an intact older
checkpoint or none; nothing half-written.
**Image build succeeds, container exits instantly with the validation error**
— the config in `/etc/harvest` isn't valid in this environment (missing
env). The exit message names the exact rule; see the env-var entry above.

## Getting more signal

`RUST_LOG=harvest=trace` — every request, skip reason, wait, sink write,
parse fallback, with the *reason* attached. Capture next to the run for
support: `harvest queue`/`/stats` JSON, `audit.json` (+`.sig`), history
tail (`harvest history --n 5`), and the exact `config_sha256` from
`/recent` — those four identify a run's behavior without any raw data.
