### `GET /health` — liveness (auth-exempt by design)

Fixed content, safe for Docker/K8s probes:

```json
{"status":"ok","running":true}
```
