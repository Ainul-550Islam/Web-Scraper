# Buyer quickstart

Shortest path from clone to a running system with the console open. Every
command is the literal one; no placeholders other than a shell comment.

```sh
# 1. build the engine + CLI (release profile recommended)
cargo build --release

# 2. start the deterministic demo website (mock target, loopback only)
python3 demo/server.py 8099 &

# 3. one-shot demo run — writes out/basic.jsonl, prints a summary
./target/release/harvest run --config examples/config.basic.json

# 4. inspect the data you just collected
head -3 out/basic.jsonl

# 5. keep the engine serving the admin API, then open the console
./target/release/harvest serve --config examples/config.basic.json &
cd web && npm ci && npm run build && npm run preview
```

Notes:

- `serve` exposes the admin API + Prometheus text at the `metrics_addr`
  from the config (the demo uses `127.0.0.1:9678`); `Ctrl-C` stops it
  gracefully after the current cycle.
- The console dev server proxies `/api` to that address; no CORS setup and
  no token needed on loopback unless you set `admin_token` yourself.
- To verify the shipped demo data flow end-to-end the same way CI does:
  `npm run smoke` (needs the engine on 9678) and `npm run e2e:buyer`
  (starts demo + engine itself; point `HARVEST_BIN` at your binary).
- Validate any config before running: `harvest validate --config <path>`.
- Ship a delivery: `harvest export --from out/basic.jsonl --to <dir>` then
  `harvest verify --dir <dir>` (add `--key` for HMAC attestations).

Demo vs live: the UI shows a DEMO MODE banner only when built with the demo
flag (`VITE_MOCK=1 npm run dev`) — that mode cannot reach your engine; the
live build cannot fake data (it displays honest offline states instead).

Next stops: [buyer-demo.md](buyer-demo.md) (full A–AC evaluation script),
[SECURITY.md](../SECURITY.md), [release-status.md](release-status.md),
[capability-matrix.md](capability-matrix.md).
