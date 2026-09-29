# Bounded primitives (cache, stream, repository windows)

House rule: nothing in `harvest` grows without a stated limit. This page
documents the three generic pieces that codify it — `src/cache.rs`,
`src/stream.rs`, `src/repository.rs` — each unit-tested in-file and
integration-tested in `tests/search_contract.rs`.

## `BoundedCache` — LRU + TTL

* Capacity N entries, insertion-ordered eviction (least-recently-*used* —
  reads refresh), TTL per entry checked against an injected clock
  (`ManualClock` in tests; no wall-clock flake).
* `capacity == 0` means **always miss** — a legitimate "disable me"
  configuration, not a crash and not an unbounded bypass.
* `stats()` returns hits/misses/evictions/expirations and `hit_ratio`
  (0.0 on zero lookups; never NaN).
* It is a cache, not a store: `get` of an expired entry removes it and
  counts an expiration; nothing is persisted, nothing is shared across
  processes.

## `Stream<T>` — bounded publish/observe

A slot per stream id with a fixed queue and an explicit overflow policy:

* `LagDropOldest` — slow readers see the newest `cap` items; every dropped
  item increments a shared counter (publishing never blocks the producer).
* `RejectNew` — the publisher of item `cap+1` is the one refused
  (returns `false`); readers keep everything already queued.
* Dropped accounting is exact: pushes beyond the capacity, counted at push
  time — the unit tests pin `dropped == 9` for 19 pushes into a cap-10
  stream.
* Closing a drained slot removes it (`closed + empty ⇒ None forever`);
  closed-but-full slots still deliver their backlog — data already accepted
  is never silently lost.

## Repository read windows

`read_tail(path, want)` is the only way any file-backed endpoint (history,
events, search) touches disk:

* bytes scanned ≤ **4 MiB** (`MAX_SCAN_BYTES`) — a larger file is refused
  with an explicit error, never silently half-read;
* records returned ≤ **512** (`MAX_RECORDS_PER_QUERY`) per query;
* `lines_scanned` counts **lines read**, which may exceed the record cap —
  the difference is reported as `truncated`, not hidden;
* malformed JSONL lines increment `malformed_lines` and are skipped;
* a missing file yields an **empty window** (disabled ≠ broken), while an
  unreadable directory or over-budget file is an `Err` — the two are never
  conflated.

## Why they live in the library

The engine's own politeness store, checkpoint files, and admin tailing
already predate and keep their semantics (see `docs/modules/`). These
primitives exist for embedders who need the same guarantees without
reaching into engine internals, and for the new read paths
(`harvest::search`, artifact loading) that had no bounded reader before.
Reuse over reinvention: `repository::Window` feeds both the search layer
and `harvest history`-style tooling; `api_models::PageEcho` is one shape,
shared with the HTTP paging vocabulary.
