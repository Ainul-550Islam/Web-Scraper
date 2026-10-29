# API Keys

| | |
|---|---|
| Route | `/api-keys` |
| Nav group | Console |
| Shortcut | — (none registered) |
| Endpoints | `GET /v1/admin/tenants`, `POST /v1/admin/tenants`, `GET /v1/admin/keys`, `POST /v1/admin/keys`, `POST /v1/admin/keys/:id/rotate`, `POST /v1/admin/keys/:id/revoke`, `POST /v1/admin/keys/:id/suspend` |
| Polling | none — the page is on-demand (read the list, create, rotate, suspend, revoke) |

Tenancy for the managed scrape API. This page issues **console-authenticated**
administrative credentials — the bearer token the operator already holds is
what authorizes every call here, never a tenant key.

## The three things a key carries

| Column | Meaning |
|---|---|
| `scopes` | which families of endpoint the key may call — see below |
| `limits` | `rps`, `burst`, `max_concurrent_jobs`, `monthly_quota`, `max_response_bytes`, `allowed_domains` |
| `status` | `active`, `suspended`, `revoked`, `expired` |

A key is a `sk_` prefix plus a random body. Only the **hash** is stored; the
plaintext is returned exactly once, in the creation response, and the console
never reads it back. Rotation issues a new plaintext and invalidates the old one
in the same transaction.

## Scopes

| Scope | Unlocks |
|---|---|
| `scrape` | `POST /v1/scrape` |
| `jobs_read` | `GET /v1/jobs`, `GET /v1/jobs/:id` |
| `jobs_write` | `POST /v1/jobs`, `POST /v1/jobs/:id/cancel` |
| `usage_read` | `GET /v1/usage` |
| `keys_admin` | the tenant's own key administration |

A key without a scope fails closed with `403` on the first call that needs it —
the server checks the scope before it does any work, so an unauthenticated
probe learns nothing about the tenant's limits.

## Lifecycle

| Action | What it does | What the server answers |
|---|---|---|
| create | insert a hash, return the plaintext once | `201` with `{ schema, api_key, key }` |
| rotate | revoke the old hash, mint a new one, return it once | `200` with `{ schema, api_key, key }` |
| suspend | stop accepting the key, keep its ledger rows | `200` with the updated key |
| revoke | same, and the key cannot be reinstated | `200` with the updated key |

Suspending or revoking is immediate: the next request with that key is refused
before it reaches the fetch path, so it consumes no quota and produces no ledger
row. Rotation and revocation are audited on the tenant's own account.

## Limits are enforced, not advisory

The limits shown on the key are the ones the server enforces per request:

- **`rps` / `burst`** — a token bucket, refilled continuously. Exceeding it is
  `429` with a `Retry-After`.
- **`max_concurrent_jobs`** — a per-key in-flight ceiling; beyond it, `429`.
- **`monthly_quota`** — billed units for the calendar month; beyond it, `403`.
- **`max_response_bytes`** — the body is truncated to this size and the truncation
  is recorded in the ledger.
- **`allowed_domains`** — an empty list allows nothing; a non-empty list is
  matched against the target host before any DNS resolution happens.

## Honest failure modes

| What happened | What you see |
|---|---|
| no/incorrect bearer token | "The admin API requires a bearer token" (401) |
| key has no scope for the call | `403` with the scope name, before any work |
| key suspended / revoked / expired | `401` with the status, and the row shows it |
| a key list read fails | "Could not read the keys" plus a `Retry` |
| no tenant selected | "No tenant selected" — create one first |

- **See also:** [modules/tenancy](../modules/tenancy.md), [modules/ratelimit](../modules/ratelimit.md), [api/26 managed scrape API](../api/26-managed-scrape-api.md)
