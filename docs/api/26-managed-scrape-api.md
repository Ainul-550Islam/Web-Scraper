# Managed scrape API (`/v1/*`)

The customer-facing surface. One listener, two authorities: the operator
console speaks the existing bearer token on `/`, `/metrics`, `/control/*` and
every other route that was already there; `/v1/*` speaks a tenant **api key**,
and `/v1/admin/*` speaks the console bearer again so the console (and only the
console) can administer tenants and keys.

Nothing here is a second engine. A `POST /v1/scrape` is validated, handed to the
same fetch/extract/transform machinery the scheduler uses, metered through an
append-only ledger, and returned in one canonical envelope. The operator
half delegates to machinery that already exists too.

## Authentication

| Surface | Credential | Where it comes from |
|---|---|---|
| `POST /v1/scrape`, `POST /v1/jobs`, `GET /v1/jobs/*`, `POST /v1/jobs/:id/cancel`, `GET /v1/usage` | `Authorization: Bearer hv_live_…` | a key from `POST /v1/admin/keys` |
| `POST /v1/admin/tenants`, `GET /v1/admin/tenants`, `POST /v1/admin/keys`, `GET /v1/admin/keys`, `POST /v1/admin/keys/:id/{rotate,revoke,suspend}`, `GET /v1/admin/usage`, `POST /v1/admin/usage/export` | the console bearer | `admin_token` in the config |
| `GET /v1/openapi.json` | none | it is the published contract |

Fail closed, always:

* No `Authorization` header → `401 missing_api_key`.
* A key that is not in the store → `401 invalid_api_key`, and the presented
  secret is never echoed back.
* A revoked key → `401 revoked_key`. An expired key → `401 expired_key`.
  Both are checked on **every** request; nothing is cached.
* A suspended key → `403 suspended_key`.
* A well-formed key without the scope the endpoint needs → `403 insufficient_scope`.

Keys are stored as SHA-256 hashes only. The plaintext exists exactly once, in
the response of `POST /v1/admin/keys` (and the `harvest keys create` output),
and is never stored, logged, echoed by another endpoint, or included in an
audit line. Lookups hash the presented secret and compare in constant time.

## Canonical envelope

Success responses carry `schema: 1` (`harvest.scrape.v1`, `harvest.job.v1`,
`harvest.error.v1`). Errors use one shape everywhere:

```json
{ "schema": 1, "error": { "code": "invalid_request", "message": "…", "details": null, "retry_after_ms": null } }
```

`code` is stable and machine-readable; `message` is for humans and may be
reworded between releases. `retry_after_ms` is set on `429` and on any
refusal that asks the caller to slow down.

Status codes in use: `200` (scrape, job read, usage), `201` (job created),
`202` (job cancelled), `400` invalid request, `401`/`403` auth, `404` unknown
route or a job that does not exist *for this tenant*, `405` wrong method,
`409` state conflict (cancelling a terminal job), `413` body over the
configured limit, `429` rate/concurrency/quota, `500` internal, `503` the
surface is not attached.

## `POST /v1/scrape`

```
POST /v1/scrape
authorization: Bearer hv_live_…
content-type: application/json
idempotency-key: <optional, 8..128 of [A-Za-z0-9_-]>

{
  "url": "https://example.com/article/1",
  "format": "text",
  "render": "auto",
  "extract": { "rules": [ { "selector": "h1", "field": "title" } ] },
  "screenshot": false,
  "session_key": "…",
  "proxy_region": "…",
  "wait": { "ms": 0 },
  "webhook_url": "https://…",
  "idempotency_key": "…"
}
```

* `url` is SSRF-checked **before anything else happens**: scheme must be
  `http`/`https`, the host is resolved, and every address must be public.
  Loopback, private, link-local, multicast, unspecified, the cloud metadata
  ranges and IPv4-mapped IPv6 forms are all refused, and every redirect hop is
  re-checked up to `max_redirects`. The guard is rebuilt per request.
* `format` is `text`, `html` or `json`. `render` is `auto`, `never` or
  `always`. `extract` reuses the project's own CSS/JSONPath rule types.
* `screenshot`, `session_key`, `proxy_region` and `wait` are the browser
  surfaces, bounded by the same config the console uses.
* The body is capped at `managed_api.max_request_bytes` (default 64 KiB);
  larger is `413` before parsing.
* Unknown request keys are refused — a typo must not become a default.

Response `200`:

```json
{
  "schema": 1, "request_id": "req-…", "status": 200,
  "url": "https://example.com/article/1", "final_url": "https://example.com/article/1",
  "content_type": "text/html", "bytes": 48211, "latency_ms": 412,
  "render": "auto", "format": "text", "body": "…", "extracted": null, "records": null,
  "artifacts": [], "usage_units": 12, "cost": "0.000120",
  "screenshot": null,
  "chain": { "seq": 17, "hash": "…" }
}
```

`usage_units` is what the ledger charges; `cost` is `units × pricing.rate`.
`chain` is the head of the usage ledger after this request, so a caller can
confirm its own entry is on the record.

### Idempotency

Send `Idempotency-Key` (header or `idempotency_key` in the body) and the first
response is replayed for the same key, tenant and body — including on a
retry after a timeout. Replays do not re-fetch and do not re-charge. Keys are
kept for `idempotency_retention_ms` and the store is bounded, so a key that
falls out of retention simply starts a new request.

## Jobs

```
POST /v1/jobs                      { "scrape": { … }, "schedule": { … } }   → 202
GET  /v1/jobs                      → 200 { "schema": 1, "jobs": [ … ], "count": n }
GET  /v1/jobs/{job_id}             → 200 JobResponse
POST /v1/jobs/{job_id}/cancel      → 202 JobResponse (409 when terminal)
```

`schedule` is `{ "interval_minutes": 0, "repeat": 1 }`; the job runs once
(`repeat`) and can be re-submitted. Jobs are bounded by
`managed_api.jobs.max_jobs` and expire after `jobs.ttl_secs`.

Job state is `queued`, `running`, `done`, `failed`, `cancelled`; the last three
are terminal. Cancelling a terminal job is `409`. Jobs are listed newest
first, capped at `V1_JOBS_MAX_LIST` (200).

A job is readable only by the tenant that created it: another tenant's valid
key gets `404`, not `403`, so the endpoint does not confirm that the id exists.

## Usage

```
GET /v1/usage?key_id=…&since=YYYY-MM-DD&until=YYYY-MM-DD&limit=100
```

Answers from the append-only ledger: per-key rows, per-day rows and totals,
plus the chain head. `since`/`until` are calendar days. `POST /v1/admin/usage/export`
writes a report (jsonl, csv or the summary csv) into `managed_api.export_dir`
and returns the manifest row — `{schema, path, manifest, rows, bytes, sha256, truncated}`.

### The ledger

Every metered action is one entry, written with an append-only file handle:

```json
{ "seq": 17, "ts_ms": 1761800000000, "tenant_id": "…", "key_id": "…",
  "action": "scrape", "units": 12, "request_id": "…", "prev_hash": "…", "hash": "…" }
```

`hash = sha256(seq || prev_hash || canonical entry)`. Tampering with an entry,
dropping an entry out of the middle, or reordering breaks the chain and
`verify()` reports where. The ledger is also the audit trail: no API response,
log line or event ever contains a key plaintext.

## Webhooks

`webhook_url` registers a signed callback, delivered after a scrape (or a job
run) finishes:

* Body is the same canonical envelope, signed with HMAC-SHA256 over
  `timestamp + "." + body` using `managed_api.webhook.signing_key`.
* Headers: `harvest-signature: t=<unix-seconds>,v1=<hex>` and
  `harvest-timestamp: <unix-seconds>`. Receivers should reject anything more
  than `webhook.max_skew_secs` old.
* Retries follow the webhook retry policy (bounded attempts, exponential
  backoff). `managed_api.webhook.allowed_hosts` restricts delivery to a host
  list; a host outside it is dead-lettered with the reason.
* The URL is re-checked with the SSRF guard **before each attempt**, so a
  redirect cannot move delivery onto a private target.
* Failures are appended to a dead-letter JSONL file under the managed root so
  a webhook that never lands is visible rather than silent.
* A webhook with no delivery key configured is refused at registration, not
  at delivery time.

## Prometheus

`GET /metrics` exposition includes the managed counters alongside the console
ones:

| Metric | What it counts |
|---|---|
| `harvest_managed_api_requests_total{status}` | every `/v1/*` request by HTTP status |
| `harvest_api_rate_limited_total` | requests refused by the token bucket or concurrency cap |
| `harvest_api_quota_denied_total` | requests refused by the monthly quota |
| `harvest_api_usage_units_total` | metered units, by key |
| `harvest_api_jobs_active` | jobs not yet terminal |
| `harvest_webhook_deliveries_total{result}` | deliveries by outcome |

## Bounds (all overridable, all validated at startup)

`src/config.rs` refuses to start with an out-of-range value, so a typo is a
startup error and not a runtime surprise.

| Bound | Default |
|---|---|
| `max_request_bytes` | 64 KiB |
| `max_redirects` | 5 |
| `max_url_len` | 2048 |
| `request_timeout_ms` | 60000 |
| `max_records` | 5000 |
| `extract_max_fields` | 64 |
| `jobs.max_jobs` | 5000 |
| `jobs.ttl_secs` | 604800 |
| `idempotency_retention_ms` | 3600000 |
| `idempotency_max_records` | 4096 |
| `max_keys_per_tenant` | 64 |
| `webhook.max_attempts` | 5 |
| `webhook.timeout_ms` | 10000 |
| `webhook_url_max_len` | 2048 |

## Operator half

```
POST   /v1/admin/tenants                     { "name": "acme" }        → 201 { tenant }
GET    /v1/admin/tenants                     → 200 { tenants, count }
POST   /v1/admin/keys                        { tenant, scopes[], limits, expires_at } → 201 { key, plaintext }
GET    /v1/admin/keys?tenant=acme            → 200 { keys, count }
POST   /v1/admin/keys/{id}/rotate           → 200 { key, plaintext, revokes_old }
POST   /v1/admin/keys/{id}/revoke           → 200 { key }
POST   /v1/admin/keys/{id}/suspend          → 200 { key }
GET    /v1/admin/usage?…                     → 200 usage report
POST   /v1/admin/usage/export                { format, since, until, key_id } → 201 manifest
GET    /v1/openapi.json                      → 200 the contract (no auth)
```

`POST /v1/admin/keys` returns the plaintext **once**. `GET /v1/admin/keys`
returns only the prefix, the hash, the scopes, the limits, the status and
`last_used_at_ms`. Rotating a key revokes the old plaintext and issues a new
one; the old one stops working on the very next request.

`managed_api.allow_private_targets` is a test/demo escape hatch that lets the
SSRF guard accept loopback and private targets so a local demo origin can be
scraped. It defaults to `false`, and `Config::validate` logs a startup warning
when it is turned on — never enable it on a host that can reach anything you
would rather not expose.

## CLI

```
harvest keys create  --config … --tenant acme [--tenant-name Acme] \
    [--scopes scrape,jobs_read,jobs_write,usage_read] [--rps 5] [--burst 10] \
    [--max-concurrent-jobs 2] [--monthly-quota 100000] [--max-bytes 16777216] \
    [--domains example.com] [--expires-at 1761800000]
harvest keys list    --config … [--tenant acme]
harvest keys rotate  --config … --key <id>
harvest keys revoke  --config … --key <id>
harvest keys suspend --config … --key <id>
harvest keys tenant  --config … --name acme

harvest usage --config … [--since 2026-10-01] [--until 2026-10-31] \
    [--key <key-id>] [--tenant <tenant-id>] [--format json|jsonl|csv]
```

`keys create` prints the plaintext exactly once; nothing else ever shows it.
`usage --format csv` writes into `managed_api.export_dir` and prints the same
manifest row the export endpoint returns.

## The contract

`GET /v1/openapi.json` renders OpenAPI 3.1 from the same route table the
router uses, and `docs/openapi.json` is that document checked in.
`tests/openapi_contract.rs` asserts they are byte-identical and that every
route the router serves appears in the document and vice versa, so a route
cannot exist undocumented — or be documented but not served.

## Tests

* `src/tenancy/**` — keys, scopes, limits, store atomicity, constant-time
  comparison, plaintext handling (58 tests).
* `src/ratelimit.rs` — token bucket, concurrency, quota, idle eviction (20).
* `src/ssrf.rs` — scheme, resolution, redirect hops, IPv6/IPv4-mapped forms
  (27).
* `src/usage/**` — ledger chain, tamper detection, aggregation, export (25).
* `src/scrape_api/**` — validation, pricing, service, idempotency, jobs,
  webhooks, openapi (30).
* `tests/managed_api.rs` — the wire contract end to end.
* `tests/tenancy_security.rs` — refusals and secret leakage.
* `tests/openapi_contract.rs` — the published contract.
* `tests/route_contract.rs` — the route table is pinned to
  `web/src/lib/apiRoutes/API_ROUTES.ts`.
