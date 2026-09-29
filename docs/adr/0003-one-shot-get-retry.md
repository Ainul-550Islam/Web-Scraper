# Retry GETs exactly once; never retry POSTs

Status: **Accepted**

## Context & decision

`fetchJson` retries only on network error, timeout, or 503 — one retry, no backoff ladder — and only for GET. The two POST routes (`/jobs/<id>/pause|resume`) are engine-side state flips, but re-sending a POST after an ambiguous timeout could double-apply operator intent against a slow engine; refusing POST retry is the honest default. A normalized `ApiError(kind: timeout|offline|http)` survives to the UI so pages show the right affordance.
