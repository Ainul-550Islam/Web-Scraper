# Jobs

| | |
|---|---|
| Route | `/jobs` |
| Nav group | Operations |
| Shortcut | g j |
| Endpoints | `/jobs`, `/events?limit=200`, `/failures?limit=50`, `/retries` |
| Polling | 5 s (jobs) · 8 s (events/failures/retries) |

Per-target tallies for the live (or latest) run: written, failed, blocked,
quarantined, revalidated — every value a plain number, so the page stays honest
between runs (the engine answers with an empty `jobs` array, not a guess).

A retry card folds `recent_delays_ms` into MiniBars; failures render through the
same row contract as the Quality page.

- **See also:** [api/08 operational views](../api/08-operational-views-jobs-runs-pipeline.md)
