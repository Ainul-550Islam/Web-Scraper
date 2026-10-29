# Usage

| | |
|---|---|
| Route | `/usage` |
| Nav group | Console |
| Shortcut | — (none registered) |
| Endpoints | `GET /v1/admin/usage`, `GET /v1/admin/usage/export` |
| Polling | 60 s while the page is open, re-issued whenever the filter changes |

Metering for the managed scrape API, read straight out of the **append-only,
hash-chained usage ledger**. Every accepted customer request writes exactly one
row, and every row carries the hash of the row before it — so the report you are
looking at is not a running counter somebody increments, it is a sequence whose
integrity is checkable.

## What the header says

`N key(s) · M requests · U units` — the totals for the whole window, with the
ledger's head sequence in a status chip. The chip goes degraded if the window
contains any error row, because a window with a failed request is a window with
a gap somebody should look at.

## The three tables

| Table | One row per | Columns |
|---|---|---|
| Metrics | the window | requests, units billed, bytes delivered, succeeded, refused, errors |
| Per key | key | requests, units, bytes, ok / refused / errors, first / last seen |
| Per day | UTC calendar day in the window | requests, units, bytes |

`units` are billed units, not requests: a request costs its own work plus the
work of the resources it touched. `bytes` is the delivered response size,
capped at the key's `max_response_bytes`.

## Filtering

| Control | Meaning |
|---|---|
| `Key` | a single key id, or all keys |
| `Since (UTC day)` / `Until (UTC day)` | inclusive calendar days, parsed as `T00:00:00Z` |
| `Row limit` | how many per-key rows to return (default 200, the server's own ceiling) |

Filtering happens server-side, so the ledger is scanned once for the window and
the page never downloads the whole history to sort it locally. **Apply** issues a
new read; **Reset** clears the filter and re-reads. Changing a filter aborts the
in-flight read and hides stale rows rather than showing the old window under a new
heading.

## Export

`Export JSONL` and `Export CSV` download the same window through
`GET /v1/admin/usage/export`. Each file is a `File` / `Manifest` / `Rows` triple:
the manifest carries the window, the filter and the ledger head sequence the
rows were read under, so an export can be reconciled against the ledger later
rather than trusted on its own.

## Honest failure modes

| What happened | What you see |
|---|---|
| window contains no rows | "No usage in this window" (metric cards still render zeroes) |
| window has rows but no day buckets | "No days in this window" |
| the read fails | "Could not read the usage report" plus a `Retry` |
| a truncation happened | the row says so, and the byte total is the capped value |

- **See also:** [modules/usage](../modules/usage.md), [modules/tenancy](../modules/tenancy.md), [api/26 managed scrape API](../api/26-managed-scrape-api.md)
