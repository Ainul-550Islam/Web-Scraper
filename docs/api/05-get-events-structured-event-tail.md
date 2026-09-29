### `GET /events` — structured event tail

JSON tail of the bounded in-memory event ring (newest `limit` events,
default 64, max 512). The same events are streamed to `events_path` as
JSONL for the whole run; this endpoint is for live tailing without a file.

```json
{
  "limit": 2, "ring_len": 182, "dropped": 0, "last_seq": 182,
  "events": [
    { "seq": 181, "ts": 1789664080, "kind": "sink_write_completed",
      "job": 41, "detail": { "url": "http://127.0.0.1:8080/p/9",
                             "fingerprint": "9f3a…" } },
    { "seq": 182, "ts": 1789664080, "kind": "job_completed",
      "job": 41, "detail": { "url": "http://127.0.0.1:8080/p/9", "target": 0,
                             "total_ms": 214 } }
  ]
}
```

Kinds: `job_submitted`, `job_started`, `request_started`, `proxy_selected`,
`request_finished`, `retry_scheduled`, `proxy_failure`,
`extraction_completed`, `sink_write_completed`, `job_completed`,
`job_failed`, `job_expired`, `queue_overflow`, `breaker_opened`,
`breaker_closed`, `record_quarantined`, `shutdown_started`,
`shutdown_completed`. `seq` is
monotonic per run; gaps in a tailed stream mean the ring dropped old
events (`dropped` counts them) — the JSONL file never skips.
