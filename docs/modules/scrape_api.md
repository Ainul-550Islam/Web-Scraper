# Module `scrape_api`

Source: [`src/scrape_api/`](../../src/scrape_api/) · unit tests: [`src/scrape_api/tests.rs`](../../src/scrape_api/tests.rs) · barrel: [`src/scrape_api.rs`](../../src/scrape_api.rs)

The managed customer API: `/v1/scrape`, `/v1/jobs`, `/v1/usage`, the operator
half under `/v1/admin/*`, and the OpenAPI document that describes all of it.

It is a second authority on the console's port, not a second server. The router
in `admin.rs` is async and awaits `dispatch_async`; `App` owns a dedicated
Tokio runtime only so that background work — job execution and webhook
delivery — can be scheduled on it. Nothing calls `block_on` from inside a
runtime.

Public surface:

- `src/scrape_api.rs`
  - `struct App` — `new`, `config`, `managed`, `store`, `limiter`, `jobs`,
    `idempotency`, `ledger`, `metrics`, `ledger_head`, `rt`
  - `struct AppInputs` — `from_config`
  - `fn time_now_ms`
- `src/scrape_api/models.rs` — `ScrapeRequest`, `ExtractSpec`, `WaitSpec`,
  `OutputFormat`, `CreateJobRequest`, `JobSchedule`, `JobState`, `ScrapeResponse`,
  `JobCreateResponse`, `JobResponse`, `ErrorBody`, `ErrorResponse`,
  `ArtifactRef`, `ChainHead`, `RESPONSE_SCHEMA`, `WEBHOOK_SIGNATURE_HEADER`,
  `WEBHOOK_TIMESTAMP_HEADER`
- `src/scrape_api/handlers.rs` — `Head`, `dispatch`, `dispatch_async`
- `src/scrape_api/validate.rs` — `validate_request`, `validate_url`,
  `validate_webhook_url`, `ssrf_error`, `ERR_INVALID_REQUEST`,
  `ERR_URL_REJECTED`, `ERR_WEBHOOK_INVALID`
- `src/scrape_api/service.rs` — `ScrapeService` (`new`, `with_browser`,
  `metrics`, `managed`, `estimate_units`, `body_for_format`), `ScrapeOutcome`,
  `ScrapeError`, `html_to_text`, `html_to_markdown`, `USER_AGENT`
- `src/scrape_api/pricing.rs` — `Pricing` (`from_cfg`, `units_for`, `cost`)
- `src/scrape_api/idempotency.rs` — `IdempotencyStore` (`open`, `lookup`,
  `store`, `put_if_absent`, `len`, `is_empty`), `Record`, `Insert`
- `src/scrape_api/jobs.rs` — `JobRegistry` (`open`, `create`, `get`, `update`,
  `transition`, `cancel`, `pending`, `len`, `is_empty`, `exhausted`), `JobRecord`
- `src/scrape_api/webhook.rs` — `WebhookSender`, `Delivery`, `DeliveryResult`,
  `sign`, `verify`
- `src/scrape_api/openapi.rs` — `spec`, `render`, `validate`, `config_summary`,
  `summary`, `config_example`, `content_type`, `signature_header`,
  `timestamp_header`, `OPENAPI_VERSION`

### Request flow

```
admin.rs handle()
  └─ route_shape("/v1/...")
       └─ dispatch_managed()          ← any /v1/*, known or not
            └─ handlers::dispatch_async()
                 ├─ ("POST"|"GET", "admin")   → admin_dispatch   (console bearer, checked upstream)
                 ├─ ("GET", "openapi.json")   → openapi::render (no auth)
                 └─ everything else           → customer_async  (api key)
```

`customer_async` authenticates, checks the scope, then:

1. checks the monthly quota (`429` before any parsing work),
2. parses and validates the body — bounded by `max_request_bytes`, unknown
   keys refused,
3. validates the URL — scheme, length, SSRF, and the webhook URL,
4. checks the domain allowlist against the parsed host,
5. replays or stores an idempotency key,
6. acquires a rate/concurrency permit,
7. runs the request through `ScrapeService` — politeness, robots.txt, circuit
   breaker, proxy selection, the same fetch/extract/transform machinery the
   scheduler uses,
8. meters the result into the ledger,
9. signs and delivers a webhook if one was registered,
10. returns the canonical envelope.

Every step that can fail returns the managed error envelope; nothing falls
through to the console's envelope.

### Jobs

`POST /v1/jobs` validates, enqueues into `JobRegistry` and returns `202` with
the job id. The worker is spawned on the app's runtime; it transitions the job
through `queued → running → done|failed|cancelled`, records the same ledger
entry a synchronous scrape would, and delivers the webhook. Reads and cancels
are tenant-scoped: another tenant's job is a `404`, never a `403`.

### Webhooks

`webhook_url` is validated at registration — `https` only unless
`allow_private_targets` is on, and the host must be in `webhook.allowed_hosts`
when that list is non-empty. Delivery is HMAC-SHA256 over
`<unix-seconds>.<body>`, bounded by the timestamp's skew window, retried with
bounded attempts and exponential backoff, re-checked against the SSRF guard
before each attempt, and dead-lettered to a JSONL file when it cannot land.

### The contract

`openapi::render` builds the document from the same route table the router
uses, so a route cannot exist undocumented. `docs/openapi.json` is that
document checked in, and `tests/openapi_contract.rs` asserts the served copy and
the checked-in copy are byte-identical — plus that every operator route is
absent from the public document, since `/v1/admin/*` is console-bearer only.
