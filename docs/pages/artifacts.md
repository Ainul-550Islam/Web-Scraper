# Artifacts

| | |
|---|---|
| Route | `/artifacts` (optionally `?artifact=art-3` to inspect one) |
| Nav group | Data & Guardrails |
| Shortcut | — (none registered) |
| Endpoints | `GET /control/jobs`, `GET /control/artifacts/:id`, `GET /artifacts/:id`, `GET /artifacts/:id/verify` |
| Polling | 10 s for the job list; 15 s for a single artifact's metadata |

What jobs produced, and whether it can still be trusted.

## Where the list comes from

There is no artifact-index endpoint, so the page derives its rows from the
retained job list (`GET /control/jobs`), which embeds each job's artifacts.
It says so on the page rather than implying a second source of truth.

## Actions

| Action | Call | What the server does |
|---|---|---|
| Verify | `GET /artifacts/:id/verify` | re-runs the manifest + checksum comparison (the same logic `harvest verify` uses) and re-marks every artifact of that delivery |
| Download | `GET /artifacts/:id` | streams the bytes with `content-length`, `x-artifact-sha256` and a bare `content-disposition` filename — or `409` if the delivery no longer verifies |

Authentication is a header, never a query parameter, so the token cannot end up
in a URL, a referer or an access log.

## Run output is different

A crawl registers the configured output file as an artifact with the
`run_output` role. It downloads — it is a real, registered file — but it has no
manifest to check, so **Verify is not offered** for it. `run_output` is real
and honest, not an error state.

## Safety, stated on the page

- Ids are validated server-side: anything that is not `art-<n>` is a `404`
  before a filesystem call happens, so no path can be walked out of the
  delivery directory.
- An artifact whose delivery failed verification stops being downloadable
  (`409`); the console disables the button, but the server is what refuses.
- Size is capped server-side before a byte is read.
