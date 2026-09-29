## Admin HTTP API

Served on `metrics_addr` (default `127.0.0.1:9678`; set `""` to disable the
admin server entirely). HTTP/1.1, `connection: close`, no keep-alive
assumptions; the request line is parsed for the path only (every route is a
read; there are no write endpoints, and a body is never consumed). All
responses include:

```
content-type: application/json | text/plain | text/html (+ CSP for HTML)
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
anything else — malformed, oversized, folding attempts — is replaced with
a generated `req-…` id, never echoed back raw. The same id keys structured
error bodies, the operator audit line, and the `harvest::api` log record
for that request.

**Errors** share one JSON envelope — no paths, stack traces, or config in
it, ever:

```json
{"error": {"code": "invalid_request", "message": "…"}, "request_id": "req-0142…"}
```

status → code: `401 unauthorized` (constant message; never reveals whether
a token is configured), `400 invalid_request` (bad query, bad body,
illegal target bytes), `404 not_found` (plain-text route listing),
`405 method_not_allowed`, `413 payload_too_large` (POST bodies are capped
at 4 KiB — `MAX_BODY`), `503 unavailable`, `500 internal`. All list endpoints are
bounded: `limit` defaults to 50 and clamps to 1–200, `offset` maxes at
1,000,000 — a request can never force an unbounded read or allocation.

**Authentication.** If `admin_token` is set in the config, every route
except `GET /health` requires `Authorization: Bearer <token>` (timing-safe
compare; the token may also be injected as `"${HARVEST_ADMIN_TOKEN}"`).
Wrong/missing token ⇒

```
HTTP/1.1 401 Unauthorized
www-authenticate: Bearer
```
with the constant body `authorization required: send 'Authorization: Bearer
<token>'` — it never echoes the path, the presented header, or any part of
the expected token. If no token is configured the routes are open — and
`validate()` only allows that on a loopback bind (routable/`0.0.0.0` bind
without a token fails at startup).
