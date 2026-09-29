### `GET /recent` — newest run-history entries

Array (newest first, max 20) of the lines appended to
`schedule.history_path`. Each entry:

```json
{
  "config_sha256": "c186b56acb3d0f89c44e6e374e2816d7687ae99398134256746041f1774f8037",
  "duration_ms": 3007,
  "respect_robots": true,
  "run": "demo-run",
  "status": "ok",
  "summary": {
    "blocked": 0,
    "ckpt_writes": 0,
    "deduped": 0,
    "duration_ms": 3007,
    "pages_fetched": 4,
    "per_target": {
      "demo-catalog": {
        "blocked": 0,
        "failed": 0,
        "revalidated": 0,
        "written": 3
      },
      "demo-news": {
        "blocked": 0,
        "failed": 0,
        "revalidated": 0,
        "written": 1
      }
    },
    "records": 4,
    "requests_total": 4,
    "retries": 0,
    "revalidated": 0,
    "robots_denied": 0,
    "written": 4
  },
  "targets": 2,
  "ts": 1789663969
}
```

`config_sha256` is the policy hash that `harvest export` embeds in client
manifests — one value links runs ↔ deliveries.
