# Command Center

| | |
|---|---|
| Route | `/command` |
| Nav group | Operations |
| Shortcut | — (none registered) |
| Endpoints | `/runtime` `/storage` `/alerts` `/events?limit=64` `/runs?limit=1` |
| Polling | 5 s (runtime, events) · 15 s (storage, alerts, runs) |

One-glance operations screen for the live engine. Composes four real
snapshots — nothing is inferred and no endpoint is invented:

* **runtime** — `GET /runtime` when a run is active (engine name, status,
  queue depth/inflight, jobs/s, records written); between runs the route
  answers `{"error":"no run active"}` and the header says **idle** — the
  Written card then falls back to the run ledger count (`/runs` total),
  labeled `runs in ledger`, never zeroed-out theater.
* **storage lanes** — the `GET /storage` observatory in the fixed lane order
  (output, history, events, audit, checkpoint, revalidation, quarantine);
  `configured && !exists` adds a red caption naming the lanes.
* **alerts** — live item count + fired/notified from `GET /alerts`.
* **recent activity** — newest 60 events from the ring via the shared
  `ActivityTimeline` pattern; ring drops are shown, not smoothed over.

Redaction: none of these payloads carry secrets today, but the event
inspector on `/events` (which shares the timeline) masks before render.

- **Source:** `web/src/features/command/CommandCenter.tsx`
- **Tests:** `web/src/test/command-center.test.tsx` (fixture-backed, incl.
  the idle and missing-lane scenarios)
- **See also:** [storage](storage.md) · [events](events.md) · [alerts](alerts.md)
