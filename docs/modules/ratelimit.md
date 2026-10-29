# Module `ratelimit`

Source: [`src/ratelimit.rs`](../../src/ratelimit.rs) · unit tests: [`src/ratelimit/tests.rs`](../../src/ratelimit/tests.rs)

Three independent limits on one key, all evaluated before a request is
allowed to do any work, and all reported with a `Retry-After`:

| Limit | Enforced by | Exceeded gives |
|---|---|---|
| Requests per second (token bucket, with a burst) | `acquire` | `RateLimited` |
| In-flight requests (concurrency cap) | `acquire` / `Permit` | `ConcurrencyLimited` |
| Requests this calendar month (quota) | `check_quota_for` | `QuotaExhausted` |

Plus the domain allowlist, which is checked before any of these — it is the
one limit that is not a rate.

Public surface:

- `struct RateLimiter` — `new`, `with_clock`, `for_tests`, `set_bounds`,
  `idle_ttl_ms`, `max_keys`, `now`, `tracked_keys`, `retry_after_secs`,
  `acquire`, `release_concurrency`, `inflight`, `tokens_available`,
  `check_quota`, `check_quota_for`, `note_usage`, `used_this_month`, `seed_used`,
  `drop_key`
- `struct Permit<'a>` — RAII. Dropping it returns the concurrency slot; it is
  panic-safe, so a handler that unwinds cannot leak a slot and starve the key.
- `enum RateLimitError` — `RateLimited`, `ConcurrencyLimited`, `QuotaExhausted`;
  `retry_after`, `stable_code`, `message`
- `type Clock` — injectable wall clock, so a test can advance time
- `fn retry_after_duration`
- `const DEFAULT_IDLE_TTL_MS`, `const DEFAULT_MAX_KEYS`

The limiter is keyed by api-key id, not by tenant, so a tenant with many keys
gets each key's own budget. Keys that have not been seen for `idle_ttl_ms` are
evicted lazily: an idle limiter does not grow without bound, and an idle key
cannot be used to keep a slot warm forever.

`seed_used` back-fills the monthly counter from the ledger when a key is
created after the month started, so a fresh key is not handed a quota the
tenant already spent.
