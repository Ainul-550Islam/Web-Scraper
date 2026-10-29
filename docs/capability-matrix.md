# Capability matrix — every cell has a file you can open and a test you can run

Legend: **core** = default build; **feature** = needs the named cargo
feature; **console** = React UI view; evidence = real paths. "Verified" means
a test exists that exercises it (run `cargo test --all-features` /
`cd web && npm test`).

| Capability | Where | Status | Evidence (code) | Evidence (test) |
|---|---|---|---|---|
| Robots.txt fetch + obey, cached | `src/robots.rs` | core | robots.rs | src/robots/tests.rs; `robots_denials` counter in `/stats` |
| Per-domain polite delay | `src/policy.rs`, `src/sched.rs` | core | policy.rs | src/policy/tests.rs, src/sched/tests.rs |
| Page/record/time budgets | `src/budget.rs` | core | budget.rs | tests/budgets.rs; `budget_denied_*` counters |
| Circuit breakers per domain | `src/breaker.rs` | core | breaker.rs | src/breaker/tests.rs; `GET /runtime` `breakers` map |
| Retry w/ jittered backoff | `src/retry.rs`, `src/fetcher.rs` | core | retry.rs | src/retry/tests.rs; `GET /retries` |
| Conditional revalidation (ETag/If-Modified-Since) | `src/reval.rs` | core | reval.rs | src/reval/tests.rs; `requests_revalidated` counter |
| Proxy pool + failover + cooldown buckets | `src/proxy.rs` | core | proxy.rs | src/proxy/tests.rs; `GET /proxies` |
| JSONPath + CSS extraction | `src/extract.rs` | core | extract.rs | src/extract/tests.rs; Target Builder selectors |
| Transform pipeline (typed ops) | `src/transform.rs` | core | transform.rs | src/transform/tests.rs; `web/src/lib/configSchema/TRANSFORM_OPS.ts` |
| Schema enforce + drift events | `src/schema.rs` | core | schema.rs | src/schema/tests.rs; `schema_drifts` counter |
| Dedupe by content fingerprint | `src/ids.rs`, sink path | core | ids.rs | src/ids/tests.rs; `records_deduped` counter |
| Quarantine instead of dropping | `src/sink.rs` | core | sink.rs | src/sink/tests.rs; `records_quarantined`, `out/quarantine.*` |
| JSONL sink | `src/sink.rs` | core | sink.rs | tests/e2e.rs writes+reads real file |
| SQLite sink | `src/sink.rs` | feature `sqlite` | sink.rs | cargo test --all-features (sink tests) |
| Postgres sink | `src/pgsink.rs` | feature `postgres` | pgsink.rs | tests/pg.rs (service-gated in CI) |
| Redis-backed queue + multi-process workers | `src/exec.rs`, `src/supervisor.rs` | feature `redis` | exec.rs | tests/engine_layer.rs + CI all-features job |
| Checkpoint / resume with policy-sha guard | `src/ckpt.rs` | core | ckpt.rs | src/ckpt/tests.rs; CLI `run --resume` |
| Run history log + `--runs` capped scheduler | `src/sched.rs` | core | sched.rs | `GET /runs`, `GET /recent`, `harvest history` |
| Structured event tail (seq + since_seq) | `src/events.rs` | core | events.rs | `GET /events`; src/events/tests.rs |
| Hash-chained audit log | `src/audit.rs` | core (opt-in path) | audit.rs | src/audit/tests.rs; `GET /audit` |
| Prometheus text exposition | `src/metrics.rs` | core | metrics.rs | curl /metrics (buyer-demo §H) |
| Alert rules + notify + ack | `src/alerts.rs` | core | alerts.rs | src/alerts/tests.rs; POST /alerts/ack (route_contract) |
| Ops annotations | `src/ops.rs` | core | ops.rs | tests/ops_api.rs; POST /ops/note |
| Admin HTTP API (28 GET + 6 POST) | `src/admin.rs` | core | admin.rs | tests/route_contract.rs pins every route |
| Export: csv+jsonl, manifest, split/per-part, resume | `src/export.rs` | core | export.rs | tests/export_parts.rs; buyer E2E §4 |
| Export: XLSX workbook (typed cells, multi-sheet) | `src/export/xlsx.rs` | core | xlsx.rs | tests/export_formats.rs (package parts, typed cells, determinism) |
| Export: PDF summary report (bounded sample) | `src/export/pdf.rs` | core | pdf.rs | tests/export_formats.rs (header/trailer, no full dump) |
| Export: Parquet (PLAIN, typed columns, stats) | `src/export/parquet.rs` | core | parquet.rs | tests/export_formats.rs (PAR1 magic, footer length, schema names) |
| Export: XML (escaped, name-safe) | `src/export/xml.rs` | core | xml.rs | tests/export_formats.rs (well-formed, skipped unsafe names) |
| Export: ZIP bundle + checksums.txt | `src/export/bundle.rs` | core | bundle.rs | tests/export_formats.rs (members, no traversal, bundle manifest) |
| Crawl planning (depth/pages/host/include-exclude/robots) | `src/crawl/mod.rs` | core | crawl/mod.rs | tests/crawl_discovery.rs (15 tests) |
| Discovery: links, pagination, canonical, alternates, feeds, sitemaps, documents, media | `src/crawl/discovery.rs` | core | crawl/discovery.rs | tests/crawl_discovery.rs |
| Document + resource metadata (no payloads) | `src/document.rs`, `src/media.rs` | core | document.rs, media.rs | tests/crawl_discovery.rs, tests/content_integrity.rs |
| Content classification (MIME/charset/size limits) | `src/content.rs` | core | content.rs | tests/content_integrity.rs (14 tests) |
| Optional AI assistance (budgeted, redacted, fail-closed) | `src/ai.rs`, `src/ai/schema.rs` | core (off by default) | ai.rs | tests/ai_pipeline.rs (21 tests, local providers only) |
| Export Center console (format/verify/download) | `web/src/pages/ExportCenter.tsx` | — | ExportCenter.tsx | export-center.test.tsx (15 tests) |
| Crawl Studio console (plan + target composer) | `web/src/pages/CrawlStudio.tsx` | — | CrawlStudio.tsx | crawl-studio.test.tsx (12 tests) |
| AI Assist console (suggestion review) | `web/src/pages/AiAssist.tsx` | — | AiAssist.tsx | ai-assist.test.tsx (13 tests) |
| HMAC attestation sign/verify (manifest + audit) | `src/sign.rs` | core | sign.rs | CI fail-closed attest smoke; `harvest attest` |
| Config validation at startup | `src/config.rs` | core | config.rs | `harvest validate`; src/config/tests.rs |
| Sitemap seeding | `src/sitemap.rs` | core | sitemap.rs | src/sitemap/tests.rs |
| Dead-letter replay | `src/runner.rs` | core | runner.rs | `harvest run --retry-dlq` |
| Graceful SIGINT/SIGTERM drain | `src/engine.rs` | core | engine.rs | tests/e2e.rs; buyer E2E last check |
| Tenant keys: SHA-256 digest at rest, scopes, status, expiry | `src/tenancy/` | core | tenancy.rs, key.rs, scope.rs | src/tenancy/tests.rs (58); tests/tenancy_security.rs (11) |
| Per-key limits: rps/burst, concurrency, monthly quota, bytes, domains | `src/tenancy/limits.rs`, `src/ratelimit.rs` | core | limits.rs, ratelimit.rs | src/ratelimit/tests.rs (20); src/tenancy/tests.rs |
| SSRF guard: loopback/private/link-local/multicast/metadata, IPv6 + mapped, every redirect hop | `src/ssrf.rs` | core | ssrf.rs | src/ssrf/tests.rs (27); tests/managed_api.rs |
| Append-only hash-chained usage ledger | `src/usage/ledger.rs` | core | ledger.rs | src/usage/tests.rs (24); tests/managed_api.rs |
| Usage aggregation (per key / per day / totals) + JSONL/CSV export | `src/usage/aggregate.rs`, `src/usage/export.rs` | core | aggregate.rs, export.rs | src/usage/tests.rs; `GET /v1/usage`, `POST /v1/admin/usage/export` |
| `POST /v1/scrape` (+ idempotent replay), `/v1/jobs*`, `/v1/usage` | `src/scrape_api/handlers.rs`, `src/scrape_api/service.rs` | core (off by default) | handlers.rs, service.rs | tests/managed_api.rs (18); tests/route_contract.rs pins every route |
| Job queue: bounded repeats, cancellation, terminal-job conflict | `src/scrape_api/jobs.rs` | core (off by default) | jobs.rs | tests/managed_api.rs; src/scrape_api/tests.rs (30) |
| HMAC-SHA256 webhooks, retry ladder, dead-letter | `src/scrape_api/webhook.rs` | core (off by default) | webhook.rs | tests/managed_api.rs; src/scrape_api/tests.rs |
| Generated OpenAPI 3.1 + checked-in byte-identical copy | `src/scrape_api/openapi.rs` | core | openapi.rs | tests/openapi_contract.rs (7); docs/openapi.json; `GET /v1/openapi.json` |
| Managed API counters (requests, rate-limited, quota, units, jobs, webhooks) | `src/metrics.rs` | core | metrics.rs | `GET /metrics`; src/scrape_api/tests.rs |
| API Keys console (one-time reveal, rotate/suspend/revoke) | `web/src/features/managed/keys/` | — | ApiKeys.tsx, KeyReveal.tsx | api-keys-page.test.tsx (9), key-reveal.test.tsx (4) |
| Usage console (filterable metering + export) | `web/src/features/managed/usage/` | — | Usage.tsx | usage-page.test.tsx (8) |
| React console (29 routes, live polling) | `web/src` | — | web/src/app/routes/ROUTES.ts | 394 Vitest tests across 129 files; 12 smoke checks; 18 buyer E2E checks |
| Product tour + first-run welcome (real data) | `web/src/components/AppShell/` | — | ProductTour.tsx, WelcomePanel.tsx | product-tour/welcome-panel tests |
| Health summary (6 groups) on Overview | `web/src/features/overview/Overview/HealthSummary.tsx` | — | HealthSummary.tsx | overview-health-summary.test.tsx |
| Quality funnel w/ separate quarantine | `web/src/features/quality/` | — | Quality.tsx | quality page test (existing suite) |
| API Explorer (manifest-pinned) | `web/src/features/api/` | — | ApiExplorer.tsx | api-explorer page + contract tests |
| Loopback-open / bearer-gated auth + redaction | `src/admin.rs` | core | admin.rs | tests/security.rs; buyer-demo §Y |

Not offered (so buyers don't hunt for it): JS-rendered browser automation
(`browser_launch` is a best-effort headless fetch hook), cloud dashboards,
GUI config POST, asymmetric artifact signing, role-based access control, and
arbitrary server-side command execution. Bounded crawl/export/AI job endpoints
are available under `/control/*`; they are not a shell or general-purpose
execution API. This build's control-plane AI provider table contains only
`deterministic-local` and `disabled`; setting `external_allowed` cannot add an
unregistered hosted provider. **Any CAPTCHA, bot-detection, fingerprint or
authentication bypass** is out of scope; the AI layer does not fetch pages.
