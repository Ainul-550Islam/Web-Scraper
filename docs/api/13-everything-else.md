### Everything else

```
HTTP/1.1 404 Not Found
body: see / /live /metrics /stats /runtime /events /recent /jobs /runs
      /failures /retries /workers /queue /proxies /pipeline /storage
      /diagnostics /alerts /ops /audit /config /health /policy
```

A known path reached with the wrong method (e.g. `GET /alerts/ack`) is a
structured `405`, not a 404 — the route exists; the verb doesn't.
