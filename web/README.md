# Harvest console (web/)

React + TypeScript operations console for the `harvest` admin API. This
README is orientation only — the binding contract lives in
[../docs/web-client.md](../docs/web-client.md), per-page guides in
[../docs/pages/](../docs/pages/), and the enforced version of every claim is
the test suite itself.

## Commands

```sh
npm ci
npm run dev                             # http://localhost:5173, proxies /api → 127.0.0.1:9678
npm run typecheck && npm run lint       # strict TS + eslint gates
npm test                                # vitest (jsdom): 118 files, 275 tests
npm run build                           # tsc + vite build → dist/ (index.html + 27 assets)
npm run smoke                           # built dist + live engine checks (needs `harvest serve`)
npm run e2e:buyer                       # demo site + engine + console + CLI flow, self-managed
```

`VITE_MOCK=1 npm run dev` runs the UI against fixtures in dev mode (banner-marked
DEMO; mocks are compiled out of production builds).

## Layout

- `src/app/` — hash router, route registry (`routes/ROUTES.ts`, 20 routes),
  app composition
- `src/features/` — one folder per page (Overview, Runs, …, ConfigStudio,
  ApiExplorer)
- `src/components/` — `ui/` primitives, `patterns/`, `AppShell/` (shell,
  palette, tour, welcome), `Charts/`, `Icons/`
- `src/lib/` — typed API client, connection provider, config schema registry
  (37 controls), API route manifest (`apiRoutes/`), response types
- `src/test/` — harness + captured fixtures (byte-preserved, see ADR 0004)

## Rules that make the console trustworthy

- The UI renders only fields that `src/admin.rs` serves; the manifest in
  `src/lib/apiRoutes/API_ROUTES.ts` is contract-tested both ways
  (`../tests/route_contract.rs` + `src/test/contract/`).
- Tokens are memory-only (ADR 0006); localStorage holds just four safe
  preference keys (theme, nav, cadence, api) + the tour dismissal flag.
- No live number is ever imitated: unanswered endpoints render “—”/offline.
