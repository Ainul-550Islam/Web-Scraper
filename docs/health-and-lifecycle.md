# Health lanes & runtime lifecycle

Two small state machines operators keep asking about: the storage health
classifier (`src/health.rs`) and the process lifecycle (`src/runtime.rs`).
Both pinned by `tests/runtime_health.rs` (7 tests).

## Health doctrine

Every dependency classification is three-way, and the distinction is the
point:

| verdict | meaning |
|---|---|
| `disabled` | not configured — **absent is not broken**; never degrades anything |
| `unavailable` | configured but the file/endpoint is missing — this is the alarm |
| `healthy` | configured and present |

`lane_checks(&cfg) -> (Vec<Value>, bool)` emits one row per storage lane in
the fixed order the console table renders (output, history, events, audit,
quarantine, checkpoint, revalidation), each row exactly
`{lane, status, detail}` — and `harvest health` in its offline branch prints
**the same rows through the same function**, so CLI and API can never drift.
Aggregation composes into the existing `HealthReport::build`.

**Degradation trigger:** among storage lanes, a quarantine lane that exists
*and contains records* flips the aggregate to degraded and sets the console
flag. A missing-but-configured lane reports `unavailable` per lane; empty or
absent quarantine is healthy; unconfigured is disabled. No hidden heuristics
beyond that.

## Lifecycle

States: `starting → running ⇄ degraded → draining → stopped`, encoded as an
`AtomicU8` (lock-free reads from every poller):

* `mark_degraded` / `clear_degraded` move only `running ⇄ degraded`
  (CAS 1↔4); other states ignore them — a draining process is not "fixed" by
  a green check.
* `begin_shutdown { reason }` latches the first drain marker: a dedicated
  `drain_marked` atomic guarantees the marker is visible even when uptime is
  literally 0 ms (the old `at_ms > 0` sentinel hid exactly that), and repeat
  calls never move `drain_at_ms`.
* `stopped` is **final** — nothing resurrects it (`begin_shutdown` on a
  stopped runtime stays stopped).
* `attach_supervisor` promotes `starting → running` (the admin surface
  proves the engine is wired, not just constructed).
* `note_shutdown_reason` accepts only `signal | schedule | admin | fault`;
  anything else is recorded as `signal` — vocabulary over garbage.

## Serialization split

`lifecycle_json()` is deliberately **separate** from `snapshot()`:
`{state, uptime_ms, started_unix, drain: {at_ms, reason} | null}`. The
`/runtime` snapshot is fixture-pinned byte-for-byte (web contract tests), so
lifecycle keys must not leak into it; new consumers read the dedicated
accessor.

`runtime_metrics::collect(runtime, stats)` builds the console's metrics block
from ~30 fixed u64 counter keys plus `per_target_written`; a missing stats
payload yields `complete: false` rather than invented zeroes, and
`storage_avg_write_ms` is derived (never read from the wire).
