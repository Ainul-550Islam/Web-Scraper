# Run Detail

| | |
|---|---|
| Route | `/run?run=<config-name>[&ts=<unix-secs>]` |
| Nav group | Operations |
| Shortcut | — (none registered) |
| Endpoints | `/runs?limit=64&order=desc&sort=ts` `/recent` |
| Polling | 15 s |

Deep-dive for one entry of the run ledger, addressed by query parameter
(the hash router's `?run=` — same mechanism the Jobs drawer uses for `?job=`).

Lookup contract:

1. Fetch the newest 64 ledger entries (`GET /runs`), newest-first.
2. Optional `&ts=` pins one entry when a config name has several runs;
   otherwise the newest match wins.
3. Enrich with the full summary block from `GET /recent` — matched on
   `run + ts`, preferring the exact `index` triple when both sides carry it.
   If `/recent` has rotated the row out, the page says so and keeps rendering
   the ledger fields; it never blanks the screen on the missing half.

No match, no `?run=`, or an unreachable ledger are three different honest
states ("Run not found" vs "Select a run to inspect" vs "Run history did not
answer"), each with its own copy and a link back to the full table.

Shown per run: duration (human + exact ms), written, quarantined, error
(if the ledger recorded one), `config_sha256` (full on hover, `fmtHash`
prefix in the header), and the per-target tally table (written/quarantined/
failed/blocked/revalidated) straight from the engine's `Summary`.

- **Source:** `web/src/features/runs/RunDetail.tsx`
- **Tests:** `web/src/test/final-pages.test.tsx` (hit/miss/enrich paths on
  recorded fixtures)
- **See also:** [runs](runs.md) · [jobs](jobs.md)
