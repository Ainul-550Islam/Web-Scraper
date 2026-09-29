# Web console (`web/`)

A first-class React + TypeScript operations console for the harvest admin
API. It is deliberately a **pure read-mostly client**: every surface maps to
routes the engine actually serves. There is no invented backend.

```
web/
├── src/
│   ├── lib/          api.ts (typed client), types.ts (mirror of the API),
│   │                 format.ts (display + maskSecrets), configSchema.ts
│   │                 (37-control registry + 23-op transform catalog),
│   │                 connection.tsx, mock.ts (dev-only fixtures)
│   ├── hooks/        usePolled (live/refreshing/paused/offline, query-key
│   │                 restart), useDebounced, useTheme, useToasts
│   ├── components/   AppShell (incl. named `AppShell` frame, Sidebar,
│   │                 TopBar, StatusBar, palette items), Table (DataTable),
│   │                 ui (33 primitives), patterns (FilterBar, SearchInput,
│   │                 Tabs, Tooltip, Checkbox, Timeline, ChartCard,
│   │                 FormSection, PipelineNode/Connector + canonical
│   │                 aliases), Overlays (Modal/Drawer/Confirm/Toasts/
│   │                 CommandPalette), Icons (single inline-SVG registry)
│   ├── app/          hash router + route registry (26 pages, lazy chunks)
│   ├── features/     one folder per page (Overview … ConfigStudio, ApiExplorer)
│   └── test/         vitest harness + fixtures captured from the live API
├── vite.config.ts    dev proxy /api → admin addr, manualChunks, test block
└── package.json      scripts: dev / build / typecheck / lint / test
```

The structure follows the spec's `app/ components/ features/ hooks/ lib/
styles/` split; API client + types live under `src/lib/` rather than a
one-file `api/`/`types/` folder — same separation, fewer directories.

## Setup & environment variables

```sh
cd web
npm install
npm run dev                 # http://localhost:5173
```

| Variable        | Where            | Meaning                                                          |
|-----------------|------------------|------------------------------------------------------------------|
| `HARVEST_API`   | dev (vite proxy) | Admin API origin the `/api` prefix proxies to. Default `http://127.0.0.1:9678`. |
| `VITE_API_BASE` | build/dev (vite) | Request base. Default: `/api` in dev (proxied), `""` (same-origin) in production. |
| `VITE_MOCK`     | dev only         | `"1"` activates deterministic mock fixtures (hard-gated to `import.meta.env.DEV`; a visible banner never lets it pass for live data). |
| `VITE_ALLOWED_HOST` | dev          | Extra host accepted by the dev server (sandbox/preview proxies). Defaults to allowing all hosts while in the dev server only. |

Production build:

```sh
npm run build               # tsc --noEmit && vite build  → web/dist
```

Because the admin API has no static-file route, `web/dist` must be served by
something same-origin with the admin address (serve both under one origin;
e.g. nginx `location /api/ { proxy_pass http://127.0.0.1:9678/; }` — the
trailing slash strips the prefix exactly like the vite proxy). `npm run
preview` covers the same-origin contract for LAN use (bind `--host`).

## Security model (mirrors the server)

- The **bearer token lives only in module memory** (`lib/api.ts`). Never
  localStorage, never the URL, never rendered, never logged, never present
  in "copy diagnostics". The connection panel shows *set / not set* only,
  and a password-type input is the only place it is ever typed.
- Secret-valued config fields (`admin_token`, `alerts.webhook_url`, …) are
  server-redacted (`"<redacted>"`), rendered as disabled sentinels flagged
  `SECRET`, and any copy path layers `maskSecrets()` over already-redacted
  payloads.
- localStorage is used for a fixed, test-enforced allow-list of preference
  keys only — `harvest.console.theme|nav|cadence|api` + `harvest.console.tour`
  (product-tour dismissal flag) — asserted by the ux suite, since no other
  write exists.
- No remote fonts, no CDNs, no third-party requests, no HTML from the API is
  ever rendered as markup (JSON is printed in `<pre>` text nodes only). The
  status bar shows only fields the API returns (no backend version string is
  fabricated — `/runtime` does not expose one).

## Component library (spec §46 vocabulary → implementation)

| Spec name            | Implementation |
|----------------------|----------------|
| AppShell             | `components/AppShell.tsx` (`AppShell`, `Sidebar`/`MemoSidebar`, `TopBar`, `StatusBar`, `Page`) |
| PageHeader · Card · MetricCard · Button · IconButton · Skeleton · EmptyState · ErrorState · Pagination · Field · Toggle · NumberInput | `components/ui.tsx` |
| StatusBadge          | `StatusChip` (aliased as `StatusBadge` from `patterns.tsx`) |
| HealthIndicator      | `HealthCell` (aliased `HealthIndicator`) |
| Input / Textarea / Select | `TextInput` / `TextArea` / `SelectInput` (aliased) |
| Table                | `components/Table.tsx` — `DataTable`: sticky headers, client+server paging, bounded page size, sorting with `aria-sort`, density toggle, column visibility (Checkbox), roving-tabindex keyboard nav, skeleton/empty/error states, ≤640px stacked-card mode |
| FilterBar · SearchInput · Tabs · Tooltip · Checkbox · Timeline · ChartCard · FormSection · PipelineNode · PipelineConnector | `components/patterns.tsx` — each used by at least one real page (no dead components) |
| Modal · Drawer · Toast | `components/Overlays.tsx` (+ focus trap, Escape, scrim, bottom-sheet on mobile) |

## What each page binds to

| Page | Routes consumed | Behavior worth knowing |
|---|---|---|
| Overview | `/stats` `/runtime` `/diagnostics` `/events?limit=120` (+`/storage` in the health summary) | metric cards render server numbers verbatim through `fmtNum`; sparkline exists only from the live event ring; System Intelligence = derived prose from real counters, explicitly labelled "no inference"; **HealthSummary** groups System/Execution/Network/Data/Storage/Security straight from those payloads (dashes before data arrives) |
| Runs | `/runs?status&sort&order&limit&offset&since&until` `/recent` | table is the server-side window (newest 512 history lines); status/sort/order/pager/date-range all become real query params; row click → drawer with lifecycle Timeline built from the run's `summary` block |
| Jobs | `/jobs` `/events?limit=200` `/failures` `/retries` | per-target tallies with the API's own provenance note; detail drawer joins the event ring by target; only states the ring actually emits |
| Queue | `/queue` | ChartCard gauges from `per_target_active` — no history endpoint, so no fake time series; `state:"unavailable"` shown honestly |
| Workers | `/workers` | per-worker cards with a freshness bar driven by `idle_secs` (the real heartbeat signal) |
| Command Center | `/runtime` `/storage` `/alerts` `/events?limit=64` `/runs?limit=1` | one-glance live view; honest **idle** between runs (`no run active` is a state, not a failure); configured-but-missing lanes captioned red |
| Run Detail | `/runs?limit=64` `/recent` | `?run=` (+ optional `&ts=`) over the same bounded ledger window the Runs table reads; enrichment degrades gracefully when `/recent` rotated the row out |
| Proxies | `/proxies` (`/stats` fallback) | state/health/EWMA/latency/requests/failures/inflight + pool aggregates (healthy/degraded/down/unprobed, `inflight_denied`); proxies identified by **id/region only — URLs never rendered**; search + state filter |
| Activity | `/events?limit=64&since_seq[&kind]` `/failures` `/retries` | 4s delta polling against the engine's ring; client ring 140; Pause stops the loop, not the data |
| Pipeline | `/pipeline` `/stats` | read-only render of the actual engine chain via shared `PipelineNode`/`PipelineConnector`; click a node → stage detail; per-target contract table |
| Quality | `/stats` `/recent` | funnel = cumulative metrics; dedupe losses shown as negative accounting, never hidden |
| Event Stream | `/events?limit&kind&since_seq` | server params vs client narrowing labeled explicitly; selected events through the redaction-first JsonInspector (mask before display **and** copy); ring drops reported, not smoothed |
| Storage | `/storage` (+`/_backends`) | one lane row per configured sink; availability cards |
| Alerts | `/alerts`, `POST /alerts/ack?event=` | acknowledge is double-guarded (confirm modal), per entry or `*`; counts from the server response |
| Audit | `/audit?limit=200` `/ops` (+`POST /ops/note`) | `Tabs` (role=tablist) switch the two feeds; debounced `SearchInput` filters client-side over the ≤200 window (the API has no filter params); detail drawer prints the raw redacted JSON |
| Diagnostics | `/diagnostics` `/health` | expandable component cards; "copy report" = `maskSecrets(server payload)` + client-side RTT facts |
| Delivery Verify | none (local files) | browser re-hash of manifest digests via `crypto.subtle`; the CLI `harvest verify` (line counts + signature) is stated as the authority; WebCrypto-absent contexts get an honest refusal |
| Config Studio | `GET /config` | see below; a computed **StudioIndex** strip lists every section with its control count (from `CONTROLS_BY_SECTION`, 37 total), live "n edited" / "n invalid" chips, and anchor jumps to the sections |
| Config Diff | none (static) | read-only canonicalized compare of two pasted documents; LCS line diff capped at 4 000 lines; no apply button **because there is no apply endpoint** |
| Export Center | none (local files + generated command) | dataset read in-tab (20 000 record cap), bounded preview with redaction, seven format cards pinned to `harvest::export::ExportFormat`, real local JSONL prepare + digest + download, in-browser manifest re-hash; the composed `harvest export …` command is shown because **generation runs on the engine host** |
| Crawl Studio | `GET /config` | composes `CrawlCfg` + `TargetCfg` documents with the planner's ceilings (depth 20 / 1 000 000 pages / 10 000 links), read-only engine limits (concurrency, timeouts, retries, robots) from the live config; a config read failure surfaces as an error, never invented numbers |
| AI Assist | none (local files) | provider document + stage output loaded in-tab; `disabled by default`, external-consent blocking, `AI SUGGESTION` vs `VALIDATED` labelling, confidence bands, local review actions |
| Control Jobs | `GET /config`, `GET /control/jobs`, `POST /control/{crawl,export,ai}`, `POST /control/jobs/:id/cancel` | the authenticated control plane's console: one bounded command at a time, every submission idempotency-keyed, states/counters/caps straight from the server, cancel disabled against terminal jobs (the server answers 409) |
| Control Job | `GET /control/jobs/:id`, `GET /artifacts/:id`, `GET /artifacts/:id/verify` | one job: counters, limits applied, artifacts with verify/download, server timeline, audit reference; polls 2 s while the job can move, 10 min once terminal |
| Artifacts | `GET /control/jobs`, `GET /control/artifacts/:id` | artifact browser derived from the retained job list (there is no artifact-index endpoint, and none is invented); role/state filters, verify + download by registry id, never by path |
| API Explorer | none (static) | renders `lib/apiRoutes/API_ROUTES.ts` — the hand-maintained manifest of every `src/admin.rs` arm (34 routes: 28 GET + 6 POST) with auth column, real query params, owning doc file, and per-row copy-cURL; GET/POST segmented filter; pinned in both directions by `tests/route_contract.rs` + `src/test/contract/api-explorer.test.ts`, so this page cannot rot into fiction |

## Config Studio semantics (draft-only, by design)

The engine has **no config-write endpoint**. The Studio therefore is an
editor of an in-memory draft:

1. Seeds from `GET /config`; a control renders only if its key exists in the
   live document (registry: **37 controls** over 8 collapsible `FormSection`
   groups; bounds, units and help mirror `src/config.rs`/`validate()`).
2. Edits mark exactly the touched controls dirty (per-field "changed in
   draft" note + reset button). Local validation mirrors Rust `validate()`
   (ints, min/max, select options, `${ENV}` passthrough exception) plus the
   cross-field honest-UA policy; a non-`ok` server `validation` string is
   surfaced as a banner, and secret-adjacent runtime keys (bind address,
   token, attestation key, output/checkpoint paths) carry a visible
   "takes effect on restart / can drop this console" warning (§40).
3. **Download / Export draft** produce a config file with all
   server-computed keys stripped (`_*`, `config_sha256`, `validation`,
   `load_warnings`) — ready for `harvest validate` / `harvest serve`.
   **Load file** seeds a draft from such an export.
4. Per-target deep-editing opens the **Target Builder** drawer
   (`?target=<id>`): crawl bounds, include/exclude patterns, extraction
   selectors (css/jsonpath/attr/all), ordered transform chains built from
   the exact `TransformCfg` catalog (23 ops with their real argument
   surfaces — required args gate Save), and the schema contract. The
   pipeline strip is the shared `PipelineNode` graph — the same primitives
   as the Pipeline page.

## Interaction & motion contract

- Hover/focus transitions run at token speeds (`--t-fast` ≤140ms); number
  bumps animate via class toggle; shimmer only in skeletons; all of it is
  wrapped in `prefers-reduced-motion: reduce` kill-switches (checked by
  test).
- Bottom ≤640px: grids collapse, tables stack into labeled cards
  (`data-col`), drawers become bottom sheets, nav becomes a drawer opened
  from the top bar.
- The command palette (⌘/Ctrl+K, also `/`) exposes navigation to all 14
  pages plus the safe actions that exist (refresh, theme). Its search states
  plainly that it covers only loaded records — no full-text pretense.

## Tests & checks

```sh
npm run typecheck && npm run lint && npm test && npm run build
npm run build && npm run smoke   # real-dist + live-API e2e (needs `harvest serve`)
```

**89 vitest + Testing-Library tests** (9 files) mount the real pages/providers over
fixtures captured byte-for-byte from a live engine
(`src/test/fixtures/*.json`): per-page data binding, server-driven filter/
sort/paging params, cursor polling, ack POST + confirm modal, Studio dirty
tracking/validation/export stripping/secret redaction, **all 37 controls
seeded from the real document with correct input kinds**, Target Builder
transform-arg gating, command palette keyboard flow, localStorage restricted
to the four allow-listed non-sensitive preference keys, in-memory bearer
behavior, network-failure and 401 error states, loading skeletons,
focus-trapped dialogs, mobile nav, the transport contract itself (timeout,
one-shot GET retry, POST-never-retried, 429→slowdown classification,
single-flight dedup, TTL cache, probe), poller backoff on 429 against a
live clock, g-sequence shortcuts incl. input guarding, the first-run
connect panel, and a deterministic design-system contract test (tokens both themes, reduced
motion, focus-visible, stacked tables, single icon system, zero runtime
deps beyond react). No `user-event`, no router library, no storybook.

## Browser support assumptions

ES2022 build target (`vite.config.ts`); designed for evergreen
Chromium/Firefox/Safari (last two majors). Requires `:has()` for the
disabled-checkbox style (progressive decoration only), `CSS.color-mix` is
NOT used, backdrop effects fall back to opaque surfaces. Keyboard behavior
is tested in jsdom; visual states rely on `:focus-visible`.

## Troubleshooting

| Symptom | Cause → fix |
|---|---|
| 404s mentioning "see /" | Vite proxy double-prefix — hit `/api/health`, not `/api/api/health`; or the admin API is on another port → `HARVEST_API=… npm run dev`. |
| 401 on every route | Non-loopback admin bind requires a bearer token → lock icon in the top bar, paste token (memory-only). |
| Everything "Offline" | Engine not serving (`harvest serve`/`run` with `metrics_addr`), or firewall between you and the admin port. The pages keep last-known data greyed, never zeroed. |
| Dev server rejects your host header | Preview/proxy domain → start with `VITE_ALLOWED_HOST=your.host npm run dev`. |
| Blank page from `web/dist` | You opened `index.html` over `file://`, or the static host doesn't share the origin with the admin API. Serve both from one origin (nginx sample above). |
| Mock banner in a "live" build | Someone built with `VITE_MOCK=1` — production builds must not set it (banner is the canary). |

## First-run connect experience

When the console has never reached the API in the current session and the
transport itself fails, a connect panel replaces the shell: product
identity, a five-line capability summary, an **API endpoint** field
(dev default `/api` via the vite proxy; point it at
`http://127.0.0.1:9678` when serving the build elsewhere), an optional
**bearer token** field, a **Connect & probe health** button with a live
probe verdict (HTTP status, rtt, request id — normalized, never a body
dump), and an **Open console anyway** escape hatch. The engine exposes no
version string on `/health` or `/runtime`, so the panel shows none — by
design, not oversight.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `Ctrl/⌘ K` or `/` | command palette (navigation + actions) |
| `g` then `o` | Overview |
| `g` then `r` | Runs |
| `g` then `j` | Jobs |
| `g` then `w` | Workers |
| `g` then `p` | Proxies |
| `g` then `q` | Queue |
| `g` then `c` | Configuration Studio |
| `Esc` | close dialogs, drawers, palette |

Shortcuts are suppressed while a text field has focus or the palette owns
the keyboard. Single source of truth: `src/hooks/useShortcuts.ts`.

## Transport hardening (`src/lib/api.ts`)

Every request carries a timeout (15 s default, 4 s probes). Idempotent GETs
retry exactly once on network/timeout/503; `apiPost` is never retried (the
admin API has no idempotency keys). Identical concurrent GETs share one
flight and a 2 s TTL cache. A 429 makes each poller wait an extra interval
before the next tick. All failures normalize to `ApiError` with
`kind|status|code|requestId` so 400/401/404/405/413/429/500/503/timeout/
abort render consistently. `probeHealth()` powers the connect panel without
side effects.

## Demo (mock) mode

`VITE_MOCK=1 npm run dev` serves deterministic fixtures captured from a
live engine. It is compile-time gated to `import.meta.env.DEV`, and the
shell shows an unmissable **DEMO MODE** chip while active. Production
builds cannot enable it and carry no fixture payloads in the fetch path.

## Preferences and storage

Only non-sensitive preferences persist in `localStorage`:
`harvest.console.theme|nav|cadence|api`, plus the `harvest.console.tour`
skipped-or-finished flag the optional product tour writes on dismissal. The bearer token lives in module
memory only — never storage, never the URL, never logs. `Diagnostics`
exposes the refresh cadence as a control (3/6/15/30 s).

## Build, preview, smoke

`npm run build` emits per-route lazy chunks (23 page chunks ≈ 3–18 kB each;
measured 2026-09-22: entry 50.2 kB raw / 16.9 kB gzip, vendor 140.7 / 45.1,
stylesheet 37.2 / 8.1 — react + react-dom only). `vite preview` proxies `/api` exactly like dev, so
`npm run build && npm run smoke` runs a real-artifact browser-level smoke
over the served dist and a live backend (`e2e/smoke.mjs`, 12 checks: shell,
asset graph, chunk count (≥23 lazy page chunks, one per route), SPA fallback,
health, runs ledger, credential absence, 404 shape, bundle freshness). It
needs no secrets and no external services beyond `harvest serve`. The
buyer-flow E2E (`e2e/buyer.mjs`, `npm run e2e:buyer`, 18 checks) goes
further: it spawns the mock site, the engine, and the preview itself, asserts
sink-file provenance, proxy-routed API honesty (config name, redaction, 405
text), then runs the real CLI export → manifest → verify and a graceful
SIGINT drain. Both scripts are pure Node 20 stdlib — no Playwright, no grid.

## First run: welcome panel and tour (never a tutorial wall)

- **WelcomePanel** (Overview, until dismissed): backend address, health
  verdict, live capability counts (targets/proxies/robots/audit/attestation —
  all from `GET /config`), and the exact demo command behind a CopyButton.
  There is deliberately **no "run it for you" button**: the admin API has no
  run-trigger route (405 says so), so the panel copies a command instead of
  faking a mutation.
- **ProductTour** (opt-in, 15 steps, one per real route): a passive corner
  card — non-modal, never blocks the page, Back/Next navigate for real,
  Escape/Skip/Finish persist dismissal under `harvest.console.tour`. Step
  titles and descriptions *are* the `ROUTES.ts` metadata, so the tour cannot
  describe a page that does not exist.

## Known limits (honest list)

- No SSE/WebSocket: the admin API exposes none; freshness comes from
  interval/delta polling with visibility-aware back-off.
- No client-side history charts beyond what `/stats` deltas allow — the API
  keeps only a 512-line run window and a cumulative metrics map.
- The Studio cannot apply changes (see above); that is the server's choice,
  not the client's.

Styling layers live under `web/src/styles/app/` (imported in order by
`app.css`); per-glyph icons under `web/src/components/Icons/svg/`.
