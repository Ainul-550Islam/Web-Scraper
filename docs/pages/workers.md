# Workers

| | |
|---|---|
| Route | `/workers` |
| Nav group | Operations |
| Shortcut | g w |
| Endpoints | `/workers` |
| Polling | 5 s |

Pool registration, per-worker idle seconds, restarts, and utilization. The
contract keeps `busy ≤ registered` and `workers.length === registered`
([contract/workers.test.ts](../../web/src/test/contract/workers.test.ts)), so
the grid can render the snapshot directly. Between runs the endpoint returns
`state: "unavailable"` and the page shows the honest EmptyState.

- **See also:** [api/04 runtime](../api/04-get-runtime-engine-runtime-snapshot.md)
