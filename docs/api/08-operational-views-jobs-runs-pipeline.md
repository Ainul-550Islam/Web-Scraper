### Operational views (jobs, runs, pipeline)

All of these read live engine state through the runtime handle the scheduler
attaches; where data is intrinsically per-run (failures, retries, workers,
queue depth) the answer between scheduled runs is an explicit
`{"state":"unavailable", …}` — never a stale zero-filled lie.

Listings share one query grammar — `limit` (default 50, clamped 1–200),
`offset`, `sort=ts|duration|written|name|bytes`, `order=asc|desc`,
`status=ok|error|cancelled`, `since`/`until` (unix seconds, inclusive) —
and the same `PageWindow` envelope (`total`, `limit`, `offset`, `items`).
Bad values are `400`, not silent defaults.

| Route | Returns |
| --- | --- |
| `GET /jobs` | Per-target `JobInfo` list (status, attempted/succeeded/failed, pages, records, bytes, current URL) from the live runtime; between runs the tally falls back to the newest persisted run with a `note` saying so. Also embeds `runtime: RuntimeSnapshot\|null`. |
| `GET /runs` | Paged view over `schedule.history_path`: reads only the last 256 KiB / 512 lines of the file (`window` + `window_note` report the bound), then filters/sorts/pages. Each item is a `RunInfo` row (ts, run, status, written, quarantined, duration, policy hash prefix, error). |
| `GET /failures` | Newest-first failed fetches from live accounting: url host, status class, error kind, attempt, retryable. `total` + `items` capped by `limit`. |
| `GET /retries` | Retry accounting: totals from both the engine accounting ring and the run metrics, plus `recent_delays_ms` newest-first backoff delays. |
| `GET /workers` | Supervisor snapshot (per-worker id, state, current job, processed counts) wrapped in `WorkerHealth`. |
| `GET /queue` | `QueueHealth`: state, queue stats (mode, depth/max, inflight, pushed/popped/rejected/expired), avg wait ms — derived from the same snapshot, no duplicated counters. |
| `GET /proxies` | Proxy pool as `ProxyPoolInfo`: per-entry health EWMA, success EWMA, requests/failures. URLs and credentials are absent by construction. |
| `GET /pipeline` | Pipeline-stage counters: transform failures by operation, schema outcomes (ok/coerce/drift/quarantine), quarantine bytes — read from the transform registry and metrics. |
| `GET /storage` | Storage-lane health: output/history/events/quarantine/checkpoint lanes as `configured`/`exists`/`bytes`/`path_label`. `path_label` is a basename-plus-role label; raw filesystem paths never appear. |
| `GET /diagnostics` | `HealthReport` built from the component checks below — every internal health check in one document, plus a derived overall `status`. |
