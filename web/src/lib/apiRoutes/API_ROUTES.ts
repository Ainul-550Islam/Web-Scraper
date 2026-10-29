import type { ApiRoute } from "./ApiRoute";

/**
 * The admin API surface, hand-maintained 1:1 from the engine's router
 * (`src/admin.rs`, the `("METHOD", "/path")` arms) and pinned in both
 * directions: `tests/route_contract.rs` (Rust) fails if the router drifts
 * from this list, and `web/src/test/contract/api-explorer.test.ts` fails if
 * the console calls a path that is not in this list. `doc` names are files
 * in `docs/api/` of this repository (not served by the engine).
 */
export const API_ROUTES: ApiRoute[] = [
  { method: "GET", path: "/", auth: "bearer-if-configured", summary: "Built-in status dashboard (single-file HTML)", doc: "12-get-status-dashboard.md" },
  { method: "GET", path: "/health", auth: "always-open", summary: "Liveness: {status, running} — no version field is exposed", doc: "01-get-health-liveness-authexempt-by-design.md" },
  { method: "GET", path: "/live", auth: "always-open", summary: "Liveness alias for the console poller: {live: true}", doc: "03-get-stats-live-json-snapshot.md" },
  { method: "GET", path: "/metrics", auth: "bearer-if-configured", summary: "Prometheus text exposition", doc: "02-get-metrics-prometheus-text-exposition.md" },
  { method: "GET", path: "/stats", auth: "bearer-if-configured", summary: "Merged snapshot: metrics counters, proxy entries, running flag", doc: "03-get-stats-live-json-snapshot.md" },
  { method: "GET", path: "/runtime", auth: "bearer-if-configured", summary: "Engine runtime snapshot (metrics, queue, workers, accounting, breakers)", doc: "04-get-runtime-engine-runtime-snapshot.md" },
  { method: "GET", path: "/events", auth: "bearer-if-configured", summary: "Structured event ring tail (newest first by default)", params: "limit, kind, since_seq", doc: "05-get-events-structured-event-tail.md" },
  { method: "GET", path: "/jobs", auth: "bearer-if-configured", summary: "Per-target tallies of the live or latest run", doc: "08-operational-views-jobs-runs-pipeline.md" },
  { method: "GET", path: "/runs", auth: "bearer-if-configured", summary: "Paged history ledger over the newest 512 lines", params: "limit, offset, sort, order, status, since", doc: "08-operational-views-jobs-runs-pipeline.md" },
  { method: "GET", path: "/recent", auth: "bearer-if-configured", summary: "Newest run-history entries with full summary blocks", doc: "07-get-recent-newest-runhistory-entries.md" },
  { method: "GET", path: "/pipeline", auth: "bearer-if-configured", summary: "Transform/schema enforcement view per target", doc: "08-operational-views-jobs-runs-pipeline.md" },
  { method: "GET", path: "/failures", auth: "bearer-if-configured", summary: "Recent failure records from the runtime slot", params: "limit", doc: "09-internal-health-checks-distinct-from-hea.md" },
  { method: "GET", path: "/retries", auth: "bearer-if-configured", summary: "Retry counters and recent delay buckets", doc: "09-internal-health-checks-distinct-from-hea.md" },
  { method: "GET", path: "/workers", auth: "bearer-if-configured", summary: "Worker pool snapshot (unavailable between runs by design)", doc: "09-internal-health-checks-distinct-from-hea.md" },
  { method: "GET", path: "/queue", auth: "bearer-if-configured", summary: "Queue depth/inflight/wait from the live runtime", doc: "09-internal-health-checks-distinct-from-hea.md" },
  { method: "GET", path: "/proxies", auth: "bearer-if-configured", summary: "Proxy pool health with EWMAs and refusal accounting", doc: "13-everything-else.md" },
  { method: "GET", path: "/policy", auth: "bearer-if-configured", summary: "Static self-description, incl. honest 'not implemented' clauses", doc: "11-get-policy-static-selfdescription.md" },
  { method: "GET", path: "/config", auth: "bearer-if-configured", summary: "Effective config, secrets masked to null (never plaintext)", doc: "13-everything-else.md" },
  { method: "GET", path: "/storage", auth: "bearer-if-configured", summary: "Per-lane store observatory: configured/exists/bytes/kind/path", doc: "13-everything-else.md" },
  { method: "GET", path: "/diagnostics", auth: "bearer-if-configured", summary: "Component-by-component verdict with overall state", doc: "09-internal-health-checks-distinct-from-hea.md" },
  { method: "GET", path: "/alerts", auth: "bearer-if-configured", summary: "Alert engine state: fired/notified counters + live items", doc: "10-alerts-and-operator-actions.md" },
  { method: "GET", path: "/audit", auth: "bearer-if-configured", summary: "Politeness audit journal tail (only when audit_path set)", params: "limit", doc: "19-politeness-audit-format-auditjson.md" },
  { method: "GET", path: "/ops", auth: "bearer-if-configured", summary: "Operator notes feed (paired with POST /ops/note)", params: "limit", doc: "10-alerts-and-operator-actions.md" },
  { method: "POST", path: "/alerts/ack", auth: "bearer-if-configured", summary: "Acknowledge one alert kind — the only alert mutation", params: "event (query or JSON body)", doc: "10-alerts-and-operator-actions.md" },
  { method: "POST", path: "/ops/note", auth: "bearer-if-configured", summary: "Append an operator note to the /ops feed", doc: "10-alerts-and-operator-actions.md" },
  { method: "POST", path: "/control/crawl", auth: "bearer-if-configured", summary: "Start a bounded crawl job over targets declared in the attached config", params: "JSON body (op, targets, limits), Idempotency-Key", doc: "24-control-plane-execution.md" },
  { method: "POST", path: "/control/export", auth: "bearer-if-configured", summary: "Start a bounded export job over the configured run output", params: "JSON body (op, formats, dataset, bundle, verify, max_records, limits), Idempotency-Key", doc: "24-control-plane-execution.md" },
  { method: "POST", path: "/control/ai", auth: "bearer-if-configured", summary: "Start a bounded AI-assistance round through the configured provider", params: "JSON body (op, task, source, confidence, provider, external_allowed, limits), Idempotency-Key", doc: "24-control-plane-execution.md" },
  { method: "GET", path: "/control/jobs", auth: "bearer-if-configured", summary: "Paged control-plane job list with counters, artifacts and caps", params: "state, op, limit, offset", doc: "24-control-plane-execution.md" },
  { method: "GET", path: "/control/jobs/:id", auth: "bearer-if-configured", summary: "One job: state, counters, artifacts, events and audit reference", doc: "24-control-plane-execution.md" },
  { method: "POST", path: "/control/jobs/:id/cancel", auth: "bearer-if-configured", summary: "Cancel a queued or running job (terminal jobs answer 409)", doc: "24-control-plane-execution.md" },
  { method: "GET", path: "/control/artifacts/:id", auth: "bearer-if-configured", summary: "Artifact metadata and verification state — never bytes, never a path", doc: "25-artifact-downloads.md" },
  { method: "GET", path: "/artifacts/:id", auth: "bearer-if-configured", summary: "Download one finalized, verified artifact by registry id", doc: "25-artifact-downloads.md" },
  { method: "GET", path: "/artifacts/:id/verify", auth: "bearer-if-configured", summary: "Re-verify the delivery an artifact belongs to (manifest + checksums)", doc: "25-artifact-downloads.md" },

  // ---- managed customer surface (`/v1/*`) --------------------------------
  // Mirrored from `src/admin.rs::managed_route` and pinned by
  // `tests/route_contract.rs`. Customer routes authenticate with a tenant api
  // key; the `/v1/admin/*` half keeps the console bearer.
  { method: "POST", path: "/v1/scrape", auth: "api-key", summary: "Run one bounded scrape: SSRF-guarded, robots-checked, metered, one canonical response", params: "JSON body (url, format, render, extract, wait, limits, webhook_url), Idempotency-Key", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/jobs", auth: "api-key", summary: "Create a scheduled scrape job (bounded repeat/interval, durable registry)", params: "JSON body (scrape, schedule, limits, webhook_url), Idempotency-Key", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/jobs", auth: "api-key", summary: "Paged list of this tenant's jobs", params: "limit", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/jobs/:id", auth: "api-key", summary: "One job with its response, records and last error", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/jobs/:id/cancel", auth: "api-key", summary: "Cancel a queued job (terminal jobs answer 409)", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/usage", auth: "api-key", summary: "This key's metered usage plus the verified ledger head", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/openapi.json", auth: "open-contract", summary: "The OpenAPI 3.1 document generated from the route table (byte-identical to docs/openapi.json)", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/tenants", auth: "bearer-if-configured", summary: "Create a tenant (operator half of the managed surface)", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/admin/tenants", auth: "bearer-if-configured", summary: "Tenant and key counts", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/keys", auth: "bearer-if-configured", summary: "Issue an api key: SHA-256 at rest, scopes, limits, expiry; plaintext shown once", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/admin/keys", auth: "bearer-if-configured", summary: "List keys for a tenant (hashes and metadata only, never plaintext)", params: "tenant_id", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/keys/:id/rotate", auth: "bearer-if-configured", summary: "Rotate a key: revoke the old hash and issue a replacement", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/keys/:id/revoke", auth: "bearer-if-configured", summary: "Revoke a key immediately", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/keys/:id/suspend", auth: "bearer-if-configured", summary: "Suspend a key for an operator-chosen reason", doc: "26-managed-scrape-api.md" },
  { method: "GET", path: "/v1/admin/usage", auth: "bearer-if-configured", summary: "Aggregate usage across tenants (console bearer, not an api key)", params: "tenant_id, key_id, format", doc: "26-managed-scrape-api.md" },
  { method: "POST", path: "/v1/admin/usage/export", auth: "bearer-if-configured", summary: "Export the usage ledger as JSON lines or CSV into the configured export directory", doc: "26-managed-scrape-api.md" },
];
