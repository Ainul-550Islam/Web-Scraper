# Testing strategy (measured, not aspirational)

The commands below reproduce the measured test results for the reviewed tree.
Live Redis and PostgreSQL cases need their opt-in test environment variables; without them, those integration cases report their documented skips.

## The matrix

| Layer | Runner | Command | Reality check |
|-------|--------|---------|---------------|
| Engine unit tests | Cargo | `cargo test --lib --all-features` | 525 library tests passed in the current all-feature test run |
| Engine full suite | Cargo | `cargo test --all-targets --all-features` | 728 tests passed with live Redis and PostgreSQL enabled; without the opt-in service variables, their integration cases report skips |
| Console types | `tsc` | `cd web && npx tsc --noEmit` | strict mode, `noUncheckedIndexedAccess`, zero escapes |
| Console lint | `eslint` | `cd web && npx eslint .` | 0 errors, 0 warnings (barrel override is scoped, not global) |
| Console unit/dom | Vitest | `cd web && npm test` | 368 tests across 126 files — pages, UX, API, resilience, UI primitives, endpoints, and hooks |
| Build invariants | `vite build` | `cd web && npm run build` | one chunk per route asserted; demo bytes excluded from prod |
| End-to-end smoke | node script | `cd web && npm run smoke` | serves real `dist/` against live `harvest serve` :9678, 12 checks |
| Buyer flow E2E | node script | `cd web && npm run e2e:buyer` | 18 checks: spawns demo mock site + `harvest serve` + vite preview, asserts proxy-routed API honesty, then real CLI export → manifest → verify and graceful SIGINT exit 0 |
| Docs integrity | Python script | `make links` (`tools/check_links.py .`) | 302 relative links, 0 broken |

## What the vitest suites are split by

- `src/test/pages/` (9) — one file per page suite, each mounting the real
  feature page against captured fixtures via `harness.tsx`.
- `src/test/ux/` (10) — palette, shortcuts, theme, responsive nav, token
  handling, error copy, empty states, focus.
- `src/test/api/` (5) — timeout/abort, retry policy, dedupe/cache, security
  surface, connect probe; each drives `lib/api` against stubbed `fetch`.
- `src/test/resilience/` (2) — poller 429 backoff, normalized error copy.
- `src/test/components/` (56) — one file per primitive (incl. ProductTour +
  WelcomePanel): 23 `ui/` units,
  11 `patterns/` units, 8 `Overlays/` units (modal semantics, focus trap,
  palette keyboard flow), `DataTable` + `extract`, 8 `AppShell` units
  (shell chrome mounted through real providers), tones, icon registry.
  Assertions are DOM contracts only — nothing invented.
- `src/test/hooks/` (5) — useDebounced, useShortcuts/isEditable/GOTO,
  useTheme/ThemeProvider, useToasts/ToastProvider, usePolled. These found
  and fixed two source defects (offline-state stomp; `isEditable` returning
  `undefined` for non-editable elements), which is the entire point.
- `src/test/contract/` (22) — one file per fixture/endpoint pinning the JSON
  envelope (field sets, vocabularies, counters like
  `healthy+degraded+down+unprobed === total`) plus corpus rules (no `version`
  field anywhere, integer epochs). Structure is strict, volatile values are
  not — fixtures stay the captured bytes (ADR 0004).
- `src/test/target-builder/` (2) + `config-studio.test.tsx`, `command-center.test.tsx`
  + `final-pages.test.tsx` (operational-wave pages incl. JsonInspector redaction),
  `controls.test.tsx` (37 real controls — the registry is the count),
  `features/studio-index.test.tsx` (StudioIndex headline derived from
  `CONTROLS.length`, anchors ↔ `FormSection` ids), `design-system.test.ts`
  (reads every `styles/app/*.css` layer through the `@import` chain).

Shared helpers live in each directory's `helpers.ts(x)`; nothing is stubbed
in place of a real module unless the suite says so in its header comment.

## Route + repo contracts (Rust side)

- `tests/route_contract.rs` — parses every `("METHOD", "/path")` arm in
  `src/admin.rs` and every row of `web/src/lib/apiRoutes/API_ROUTES.ts`; the
  two sets must be identical, open routes are exactly `/health` + `/live`,
  and POST exists only for `/alerts/ack`, `/ops/note`, `/control/crawl`,
  `/control/export`, `/control/ai`, and `/control/jobs/:id/cancel`.
- `tests/repo_integrity.rs` — buyer-package gate: promised files exist, every
  module dir has `tests.rs`, console route count == `docs/pages/` guide
  count, fixtures corpus ≥ 20, `[package] include` is explicit.
- `web/src/test/contract/api-explorer.test.ts` — the other half: walks all
  console sources and fails if any client `apiGet/apiPost` path is missing
  from the manifest; every manifest `doc` file must exist.

## Fixtures are contracts

`web/src/test/fixtures/*.json` were captured from a running engine
(docs/adr/0004). `types.ts` must keep parsing them or tsc fails — that is the
drift tripwire. To regenerate: run `cargo run -- serve --config examples/config.basic.json`, then hit
each endpoint with the token and save the raw body. No hand-typed fixtures,
ever.

## Smoke checks (npm run smoke)

Exactly the twelve `ok()` assertions in `web/e2e/smoke.mjs`, run against a
fresh `npm run build` and a spawned `harvest serve`:

1. app shell served
2. hashed entry script referenced
3. entry chunk fetches
4. vendor chunk present
5. css bundle present
6. 14 lazy route chunks emitted
7. GET /api/health proxied → 200 ok
8. deep link /runs falls back to app shell
9. GET /api/runs returns the paged ledger object
10. GET /config never leaks credentials
11. unknown route 404s with JSON error, not HTML
12. built bundle contains connect/demo copy

## Budget floors (enforced by tests/budgets)

App source bytes, test bytes, and CSS bytes carry minimums so a future
"cleanup" cannot silently delete coverage: the current measurement is
app 277.7 kB, tests 57.1 kB, styles 51.1 kB (see `tests/budgets.rs` for
the exact assertions).

## CI

`.github/workflows/ci.yml` runs the same sequence as `make` (fmt → clippy
`-D warnings` → cargo test → tsc → eslint → vitest → build → smoke) on
`ubuntu-latest` with a warm cargo/node cache. A red gate is a red PR; there
are no per-suite opt-outs.
