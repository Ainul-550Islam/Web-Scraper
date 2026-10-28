## Admin HTTP API

The API is served on `metrics_addr` (default `127.0.0.1:9678`; set `""` to
disable the admin server). It uses a small HTTP/1.1 responder and closes each
connection. The request-header buffer is capped at 8 KiB with a 5-second
absolute header deadline. Query strings are capped at 2 KiB, 32 pairs, 64-byte
keys, and 256-byte values.

The route contract currently contains **34 routes: 28 GET and 6 POST**. Most
routes are read-only. The two operator actions are `POST /alerts/ack` and
`POST /ops/note`; the four bounded control-plane mutations are `POST
/control/crawl`, `/control/export`, `/control/ai`, and
`/control/jobs/:id/cancel`. The crawl/export/AI job submissions delegate to
the existing engine, exporter, and AI machinery; they do not expose arbitrary
command execution.

POSTs are accepted only on their documented routes and are protected by the
same bearer-authentication rule as other non-liveness routes. Cross-origin
POSTs are rejected. Bodies are read only for supported POST routes and are
bounded: `POST /alerts/ack` and `/ops/note` are limited to 4 KiB; the control
plane uses a 64 KiB limit. `Transfer-Encoding` is not supported; send a valid
`Content-Length`.

All responses carry these headers (artifact downloads use the registered
artifact's content type rather than JSON):

```
content-type: application/json | text/plain | text/html (+ CSP for HTML) | artifact media type
x-content-type-options: nosniff
cache-control: no-store
x-frame-options: DENY
referrer-policy: no-referrer
x-request-id: <correlation id>
content-length: <exact>
connection: close
```

Every response carries a correlation id: a well-formed client
`x-request-id` (1–64 bytes of `[A-Za-z0-9._:-]`) is echoed unchanged;
anything else — malformed, oversized, or containing folding attempts — is
replaced with a generated `req-…` id, never echoed back raw. The same id keys
structured error bodies, the operator audit line, and the `harvest::api` log
record for that request.

**Errors** normally use one JSON envelope; the unknown-route `404` is the
intentional exception and returns a plain-text route hint. Artifact download
success responses carry the file bytes, not an error envelope.

```json
{"error":{"code":"invalid_request","message":"limit must be a non-negative integer"},"request_id":"req-0142"}
```

Status/code pairs include `400 invalid_request`, `401 unauthorized` (constant
message; never reveals whether a token is configured), `403 forbidden`, `404
not_found` for missing registered resources (unknown paths return the plain-text
route hint), `405 method_not_allowed`, `409 conflict`, `413 payload_too_large`,
`500 internal`, and `503 unavailable`. List endpoints use route-specific
page defaults and caps; a list request cannot force an unbounded read or
allocation.

**Authentication.** If `admin_token` is set in the config, every route except
`GET /health` and `GET /live` requires `Authorization: Bearer <token>` with a
timing-safe comparison (the token may also be injected as
`"${HARVEST_ADMIN_TOKEN}"`). Wrong/missing token ⇒

```
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
```

with the constant body `authorization required: send 'Authorization: Bearer
<token>'` — it never echoes the path, the presented header, or any part of the
expected token. If no token is configured, non-liveness requests are allowed only with a
loopback `Host`; config validation also rejects a tokenless non-loopback bind.
