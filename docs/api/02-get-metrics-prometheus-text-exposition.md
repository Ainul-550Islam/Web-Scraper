### `GET /metrics` — Prometheus text exposition

```
# HELP harvest_requests_total HTTP requests started
harvest_requests_total 12
harvest_requests_ok 11
harvest_requests_retry 1
harvest_requests_blocked 0
harvest_backoff_sleep_ms_total 0
harvest_politeness_sleeps 9
harvest_politeness_permits 12
harvest_robots_fetches 1
harvest_robots_denials 2
harvest_pages_fetched 10
harvest_records_extracted 10
harvest_records_deduped 0
harvest_records_written 9
harvest_requests_revalidated 0
harvest_checkpoints_written 1
harvest_domains_abandoned 0
harvest_jobs_completed 9
harvest_jobs_failed 1
harvest_queue_rejected 0
harvest_breaker_rejected 0
harvest_retry_budget_denied 0
harvest_records_quarantined 0
harvest_schema_drifts 0
harvest_pipeline_failures 0
harvest_request_ms_total 4120
harvest_storage_writes 9
harvest_storage_write_ms_total 14
harvest_alerts_fired 1
harvest_alerts_notified 1
# HELP harvest_api_requests_total admin API requests by status class
# TYPE harvest_api_requests_total counter
harvest_api_requests_total{class="2xx"} 18
harvest_api_requests_total{class="4xx"} 2
harvest_api_requests_total{class="5xx"} 0
```

While `serve` holds a live runtime, `/metrics` appends a second block of
runtime gauges rendered from the engine snapshot (the same numbers
`/runtime` serves — no duplicated counters): `harvest_queue_depth`,
`harvest_queue_max_depth`, `harvest_queue_inflight`,
`harvest_workers_registered`, `harvest_workers_busy`,
`harvest_workers_utilization`, `harvest_workers_deaths`,
`harvest_jobs_active`, `harvest_breakers_nonclosed`,
`harvest_events_published`, `harvest_events_dropped`.

Counters and fixed-label series only; no unbounded labels — status classes
are `2xx|4xx|5xx`, nothing keyed by URL, host, token, or error string ever
appears. Scrape from Prometheus with `authorization.credentials` when a
token is set (compose wires this to `$HARVEST_ADMIN_TOKEN`).
