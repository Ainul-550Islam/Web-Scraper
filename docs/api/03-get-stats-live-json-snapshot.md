### `GET /stats` — live JSON snapshot

```json
{
  "running": true,
  "metrics": { "requests_total": 12, "requests_ok": 11, "…": "same counters as /metrics" },
  "proxies": [
    { "id": "direct", "region": null, "latency_ewma_ms": 41.7,
      "success_ewma": 1.0, "requests": 12, "failures": 0 }
  ]
}
```

Proxy **URLs and credentials are never included** — only health statistics.
