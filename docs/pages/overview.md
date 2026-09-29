# Overview

| | |
|---|---|
| Route | `/` |
| Nav group | Operations |
| Shortcut | g o |
| Endpoints | `/stats`, `/runtime`, `/diagnostics`, `/events?limit=120` |
| Polling | 5 s (metrics/events) · 8 s (runtime) · 10 s (diagnostics) |

The landing page: engine status, a per-run brief, and an "intelligence" strip
assembled from the runtime snapshot (`buildIntelligence.tsx`) — accounting rate,
requests this run, pages fetched. Metric cards use the shared tone vocabulary
(`components/ui/toneFor`), so a degraded queue can never render green.

Diagnoses are pulled here too: the overall verdict chip mirrors
`GET /diagnostics.overall`. If any endpoint 401s, `ErrorState` prompts for the
admin token rather than silently blanking the grid.

- **See also:** [api/03 stats-live](../api/03-get-stats-live-json-snapshot.md), [api/04 runtime](../api/04-get-runtime-engine-runtime-snapshot.md)
