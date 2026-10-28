### Everything else

An unknown path returns HTTP 404 with a plain-text hint listing the API
surface:

```
see / /live /metrics /stats /runtime /events /recent /jobs /runs /failures
/retries /workers /queue /proxies /pipeline /storage /diagnostics /alerts
/ops /audit /config /control/crawl /control/export /control/ai
/control/jobs /control/jobs/:id /control/jobs/:id/cancel
/control/artifacts/:id /artifacts/:id /artifacts/:id/verify /health /policy
```

A known route reached with an unsupported method (for example `GET
/alerts/ack` or `POST /stats`) returns a structured `405 method_not_allowed`
JSON error. A missing or invalid job/artifact id returns a structured `404
not_found` JSON error instead of the unknown-route hint.
