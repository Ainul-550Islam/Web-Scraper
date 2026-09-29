# Use hash routing for the console SPA

Status: **Accepted**

## Context & decision

The console ships as static files served by any HTTP server (including the engine itself via `harvest serve`) and must work from `file://`, a subpath, or a bare static host without rewrite rules. Server-side SPA rewrites are a deployment foot-gun, so routes live in the URL fragment (`#/runs`) driven by a hand-rolled `router.ts` — no router dependency. Deep links, history, and lazy per-route chunks all work within that constraint.
