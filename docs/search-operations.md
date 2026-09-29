# Search & query operations

Audience: operators scripting against the run ledger and event ring, and
contributors touching the console. Scope: the query layer in
`src/search.rs` + `src/repository.rs` + `src/api_models.rs` added in the
0.10 series.

**Library-only — no HTTP surface.** `harvest::search::search(cfg, query)`
queries the same JSONL files the admin API tails, but there is no
`GET /search` route and none is planned. The web console talks to the
existing `GET /runs` / `GET /events` endpoints with their documented
params; this module exists so CLI tooling and embedders get identical,
bounded, deterministic semantics instead of ad-hoc `jq` over raw files.

## What can be queried

| resource | file | row schema owner |
|---|---|---|
| `runs` | `schedule.history_path` (JSONL) | `harvest::runner::history_entry` — `{run, ts, status, duration_ms, config_sha256, targets, respect_robots, summary}` |
| `events` | `events_path` (JSONL) | `harvest::runtime` event bus — `{seq, ts, kind, job, detail}` |

The audit *report* (`audit_path`) is a single pretty-printed JSON document,
not a log — read it via `harvest::repository::audit_report(&cfg)` (returns
`Ok(None)` when auditing is off or the file does not exist yet: absent is
not broken).

## Hard bounds (all enforced, none configurable)

* Read window: the newest **512 lines** of the file, capped at **4 MiB** of
  bytes scanned (`repository::MAX_SCAN_BYTES`). `stats.lines_scanned`
  counts lines *read*; `PageEcho.scanned` mirrors it — rows past the record
  cap are honestly marked `truncated: true`.
* `limit` clamps to `1..=200`; `offset + limit` must stay within the 512-line
  window or parsing refuses with `"bounded window"` rather than pretending.
* Sort vocabulary is fixed per resource: runs `ts | duration`, events `ts`.
  Any other sort key, order, status, or resource is a structured
  `ApiError::invalid` (`code: "invalid_request"`), never a silent default.
* Status vocabulary is the shared `harvest::api::Status` parse — the same
  words the HTTP API accepts.
* Text search is an allowlist of scalar fields (runs: `run`, `status`;
  events: `kind`) — payload blobs are never scanned, so cost stays bounded
  and results stay predictable.

## Determinism contract

Same file bytes + same query ⇒ byte-identical rows, test-pinned
(`tests/search_contract.rs`, 9 tests): default order is newest-first over
the read window; `order=asc` is the exact deterministic reverse; **ties keep
read-window order** (line position), never a comparator accident; since/until
are inclusive on `ts`. `next_offset` is `None` at the end of the window —
there is no cursor magic past line 512.

## Embedding recipe

```rust
use harvest::api_models::SearchResource;
use harvest::search::SearchQuery;

let q = SearchQuery::from_params("runs", |k| match k {
    "limit" => Some("100".into()),
    "status" => Some("error".into()),
    _ => None,
})?;
let page = harvest::search::search(&cfg, &q)?;
// page.rows: Vec<Value>   page.page: PageEcho {limit, offset, returned, scanned,
//                          truncated, malformed_lines, next_offset}
```

`from_params` takes any lookup (query strings, env, clap) — it is transport
by injection, not a hidden parser. Malformed JSONL lines are counted
(`malformed_lines`) and skipped; a missing file is an empty page.
