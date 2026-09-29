# Queue

| | |
|---|---|
| Route | `/queue` |
| Nav group | Operations |
| Shortcut | g q |
| Endpoints | `/queue` |
| Polling | 4 s |

Depth, inflight, and wait from the live in-process scheduler. When the engine
is idle it replies `state: "unavailable"` (or an empty healthy snapshot) — the
page then shows the *real* explanation ("Queue state belongs to the active run")
instead of zeros dressed up as activity. Distributed depth is CLI-only
(`harvest queue`) and the empty state says so.

Depth over 50 flips the card to `warn`; the tone logic lives in one place
(`MetricCard` props), not in the page.

- **See also:** [api/08 operational views](../api/08-operational-views-jobs-runs-pipeline.md)
