# Runs

| | |
|---|---|
| Route | `/runs` |
| Nav group | Operations |
| Shortcut | g r |
| Endpoints | `/recent` |
| Polling | 10 s |

Paged history over the schedule ledger. The table renders the nine columns the
engine actually emits (`config_sha256, duration_ms, error, index, quarantined,
run, status, ts, written` — pinned in
[contract/runs.test.ts](../../web/src/test/contract/runs.test.ts)); `error`
renders only when non-null. Rows link to Jobs for the `index` of that run.

`window_note` from `/runs` is surfaced verbatim so nobody misreads `total` as
"all history": listings scan the newest 512 ledger lines.

- **See also:** [api/07 recent](../api/07-get-recent-newest-runhistory-entries.md), [api/15 run-history format](../api/15-runhistory-format.md)
