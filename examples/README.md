# Examples

Four configs (all pass `harvest validate`, enforced by `tests/examples.rs`), an ops request script, and real
output samples produced by a live smoke run against the demo mock — the
samples are engine output verbatim, no hand editing.

| File | What it is |
|---|---|
| `config.basic.json` | Local walk-through: crawl the bundled demo mock (`python3 demo/server.py 8099`), JSONL sink, loopback admin, no secrets. Run it immediately after building. |
| `config.production.json` | Hardened single-node: env-injected `admin_token` + alert webhook, audit + checkpoint + revalidation + DLQ, HTML and JSON targets, sqlite sink, 1 req/s pacing floor. |
| `config.pipeline.json` | Transform/schema pipeline: HTML+JSON targets with extractors, `schema` enforcement (`warn`), quarantine lane on — the shape `/pipeline` and Data Quality render. |
| `config.distributed.json` | Redis queue mode: push/worker/queue flow, dataset sink, merged with `harvest export`. Binary needs `--features redis`. |
| `api-requests.sh` | Curl cookbook for the admin API (health, auth failure, stats, metrics, recent, headers) — safe to run against a live node; prints only public shapes. |
|  `output/` (paths inside `manifest.sample.json` reference the smoke harness's own `/tmp` working dir — an artifact of how the sample was produced, not a required layout) | Real smoke artifacts: `records.sample.jsonl`, `audit.sample.json(+.sig)`, `manifest.sample.json(+.sig)` — field-level documentation lives in `docs/api.md`; these are what the docs describe. |

Validate all three offline (the production one needs dummy env values —
that's the point of `${VAR}` injection; never put the real values in the
file):

```bash
cargo run --release -- validate --config examples/config.basic.json
cargo run --release -- validate --config examples/config.pipeline.json
HARVEST_ADMIN_TOKEN=devtoken12345678 ALERT_WEBHOOK=https://hooks.example.com/harvest \
  cargo run --release -- validate --config examples/config.production.json
cargo run --release -- validate --config examples/config.distributed.json   # --features redis build for worker use; validate itself needs no redis
```

Try the signed-receipt loop on the bundled audit sample (verifies only with
the run-time key, so the expected result here is the honest failure — the
sample is proof the format parses, not proof of *your* key):

```bash
cargo run --release -- attest --gen-key /tmp/demo.key
cargo run --release -- attest --verify examples/output/manifest.sample.json --key /tmp/demo.key   # exits 1: correct fail-closed behavior
```

House rules for these files: keep placeholders on `.example`/reserved
domains, no real hostnames, no secrets, no personal paths — `validate` plus
review enforces it. If you change a config field or format, update the
matching sample in the same PR (docs ↔ examples ↔ code drift is the bug
pattern here).
