### `GET /runtime` — engine runtime snapshot

The live view of the engine layer (scheduler, supervision, transport
resilience, resource accounting, event bus health). Idle between runs of
`serve` (or when no engine runtime is attached) it answers
`{"error":"no run active"}` with 200 — a status endpoint explains itself
instead of 404-ing.

```json
{
  "engine": { "run": "demo-run", "status": "running", "uptime_ms": 124080 },
  "accounting": {
    "started_ts": 1789663969, "elapsed_ms": 124080,
    "jobs_active": 2, "jobs_submitted": 41, "jobs_completed": 37,
    "jobs_failed": 2, "jobs_expired": 0, "jobs_cancelled": 0,
    "retries": 5, "bytes_received": 812340, "requests": 39,
    "jobs_per_sec": 0.298, "avg_queue_wait_ms": 121.4,
    "recent_failures": [ { "ts": 1789664080, "url": "http://127.0.0.1:8080/x",
                           "reason": "failed:server status 503 (after 3 retries)" } ],
    "recent_retry_delays_ms": [500, 1000, 2000]
  },
  "queue": {
    "mode": "fifo", "depth": 2, "max_depth": 0, "inflight": 2,
    "per_target_depths": [[0, 2]], "per_target_active": [[0, 2]],
    "pushed": 41, "popped": 39, "rejected": 0, "expired": 0
  },
  "workers": {
    "registered": 4, "busy": 2, "utilization": 0.5,
    "total_restarts": 0, "deaths": 0, "draining": false,
    "workers": [ { "id": 0, "state": "running", "jobs_done": 10,
                   "jobs_failed": 0, "restarts": 0, "idle_secs": 0 }, "…" ]
  },
  "breakers": {
    "127.0.0.1": { "state": "open", "consecutive_failures": 3, "transitions": 4,
                   "total_opens": 1, "total_rejections": 12,
                   "last_event": "closed-to-open", "opened_ms_ago": 4210 }
  },
  "metrics": { "requests_total": 39, "…": "same display as /stats" },
  "events": { "published": 182, "dropped": 0, "ring_len": 182, "last_seq": 182 }
}
```

`breakers` lists **only non-closed hosts** (snapshots stay small). Recent
failure URLs are credential-redacted; the ring caps at 64 failure records,
so this endpoint cannot be used to grow a reader's memory.
