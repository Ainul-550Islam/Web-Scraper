# Module `tenancy`

Source: [`src/tenancy/`](../../src/tenancy/) · unit tests: [`src/tenancy/tests.rs`](../../src/tenancy/tests.rs)

Tenants, API keys, scopes, limits and the store that holds them. This is the
whole of the authentication model for the managed surface: a key is a
SHA-256 digest, a scope set, a status, a plan and an expiry, and nothing else.

The plaintext key is generated once and shown once. It is never stored, never
logged, never returned by a read, and never compared with a short-circuiting
`==`.

Public surface:

- `src/tenancy/tenant.rs`
  - `enum TenantStatus` — `active`, `suspended`; `as_str`, `parse`
  - `struct TenantId` — validated slug; `parse`, `new_random`, `as_str`
  - `struct Tenant` — `create`, `with_id`, `is_active`
  - `const TENANT_ID_MAX`, `const TENANT_NAME_MAX`
- `src/tenancy/scope.rs`
  - `enum Scope` — `scrape`, `jobs_read`, `jobs_write`, `usage_read`, `keys_admin`;
    `ALL`, `as_str`, `parse`, `allows`, `covers_jobs`, `covers_job_write`
  - `struct ScopeSet` — `parse_list`, `is_empty`, `len`, `iter`, `contains`, `allows`
- `src/tenancy/key.rs`
  - `enum KeyStatus` — `active`, `suspended`, `revoked`; `as_str`, `parse`
  - `struct KeyId` — `parse`, `new_random`, `as_str`
  - `struct GeneratedKey` — `key`, `plaintext` (the one and only time)
  - `struct ApiKey` — the persisted record; `create`, `rotate`, `is_usable`, `is_expired`, `allows`
  - `fn hash_key`, `fn verify_key`, `fn generate_key`
  - `const KEY_PREFIX`, `const KEY_MATERIAL_HEX`, `const KEY_PREFIX_DISPLAY`, `const KEY_ID_MAX`,
    `const MAX_RESPONSE_BYTES_CEILING`
- `src/tenancy/limits.rs`
  - `struct KeyLimits` — `rps`, `burst`, `max_concurrent_jobs`, `monthly_quota`,
    `max_response_bytes`, `allowed_domains`, `scopes`; `validate`, `allows_domain`,
    `retry_after_secs`, `quota_enabled`
  - `const MONTHLY_QUOTA_CEILING`, `const BURST_CEILING`, `const CONCURRENCY_CEILING`,
    `const RPS_CEILING`, `const ALLOWED_DOMAINS_MAX`, `const ALLOWED_DOMAIN_MAX`
- `src/tenancy/store.rs`
  - `struct TenancyStore` — `open`, `path`, `create_tenant`, `new_tenant`, `get_tenant`,
    `list_tenants`, `set_tenant_status`, `create_key`, `list_keys`, `get_key`, `rotate_key`,
    `revoke_key`, `suspend_key`, `set_key_status`, `set_limits`, `lookup_by_key`,
    `touch_last_used`, `stats`
  - `enum StoreError` — maps to 500/500/400/404/409
  - `const TENANCY_SCHEMA_VERSION`, `const LAST_USED_THROTTLE_SECS`, `const MAX_SNAPSHOT_BYTES`
- `src/tenancy/auth.rs`
  - `struct Principal` — `tenant_id`, `key_id`, `scopes`, `limits`, `is_admin`, `require`, `require_all`
  - `fn authenticate` — header/body credential extraction, constant-time lookup
  - `fn authenticate_headers` — the header-only entry point

Every write is atomic (`fsutil::write_private_atomic`) and the whole store is a
single bounded snapshot, so a crash mid-write cannot leave a half-applied key.
