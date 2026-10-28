# Retry GETs exactly once; never retry POSTs

Status: **Accepted**

## Context & decision

`fetchJson` retries only on network error, timeout, or 503 — one retry, no backoff ladder — and only for GET. The engine exposes six POST mutations: the operator actions `/alerts/ack` and `/ops/note`, plus the bounded control-plane operations `/control/crawl`, `/control/export`, `/control/ai`, and `/control/jobs/:id/cancel`. Re-sending a POST after an ambiguous timeout could double-apply operator intent against a slow engine, so POST is never retried. A normalized `ApiError(kind: timeout|offline|http)` survives to the UI so pages show the right affordance.
