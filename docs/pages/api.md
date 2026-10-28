# API Explorer

| | |
|---|---|
| Route | `/api` |
| Nav group | Console |
| Shortcut | — |
| Endpoints | none (renders the static manifest) |
| Polling | — |

Read-only table of the engine admin API generated from ONE source of truth:
`web/src/lib/apiRoutes/API_ROUTES.ts` — a hand-maintained manifest of every
`("METHOD", "/path")` arm in `src/admin.rs`, each row with auth mode, the
query params the router actually parses, and the owning `docs/api/NN-*.md`
file. No Swagger UI, no invented endpoints.

Anti-drift contract (both directions, so the page cannot rot):

1. `tests/route_contract.rs` parses admin.rs and the manifest and fails on
   any add/remove/rename; it also pins the auth rule (`/health` + `/live`
   open) and exactly six POST routes: `/alerts/ack`, `/ops/note`,
   `/control/crawl`, `/control/export`, `/control/ai`, and
   `/control/jobs/:id/cancel`.
2. `web/src/test/contract/api-explorer.test.ts` walks all console sources and
   fails if any `apiGet/apiPost` path is missing from the manifest, checks
   auth-column consistency, and requires every `doc` file to exist.

The Segmented control filters GET/POST; rows carry a CopyButton producing the
exact `curl` for that route. Filter state is URL-persisted like other list
pages (`?kind=POST`). See also [docs/api/00-admin-http-api.md](../api/00-admin-http-api.md).
