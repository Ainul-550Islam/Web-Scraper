# Changelog

## [Unreleased]

### Authenticated control plane (0.10.0 tree)
- **New library modules** (each with inline unit suites, wired into `src/lib.rs`):
  `control` (the typed command model: commands, ids, limits and their ceilings,
  idempotency keys, job/admission/artifact errors), `download` (the artifact
  registry: registration, metadata, verification state, and a path-safe
  size-capped opener), `export_job` (the existing exporter wrapped as a job:
  source discipline, byte cap, verification), `ai_job` (the existing AI client
  wrapped as a job: provider table, budgets, thresholds, no writes), and
  `execution` (capacity, deadline, cancellation, audit and deterministic
  state transitions). No second engine, queue, retry engine, auth system,
  export implementation or polling hook was added — every one of those reuses
  the machinery that already existed.
- **Admin API** (`src/admin.rs`): ten new routes, all fail-closed on the bearer
  token (`/health` and `/live` remain the only open routes):
  `POST /control/{crawl,export,ai}`, `GET /control/jobs`,
  `GET /control/jobs/:id`, `POST /control/jobs/:id/cancel`,
  `GET /control/artifacts/:id`, `GET /artifacts/:id`,
  `GET /artifacts/:id/verify`. Mutating submissions accept an
  `Idempotency-Key` (same key + same body ⇒ the original job with
  `replayed: true`; same key + different body ⇒ 409). Command bodies are capped
  at 64 KiB. Responses carry counters, limits, artifact *names* and hashes —
  never a filesystem path, a payload or a token. `status_line` learned 202.
- **Audit**: `AuditLedger::note_with_detail` adds a bounded, sanitized `detail`
  to a ledger line; an AI submission records the provider it named and whether
  external processing was consented to.
- **Console** 23 → 26 routes: **Control Jobs** (compose one bounded command,
  idempotency key shown, server caps displayed, cancel disabled against
  terminal jobs), **Control Job** (one job's counters, applied limits,
  artifacts with verify/download, server timeline, audit reference) and
  **Artifacts** (derived from the retained job list, filtered by role/state,
  verified and downloaded by registry id). Components: `JobStateBadge`,
  `CommandComposer`, `ArtifactList`. `lib/api.ts` gained idempotency-key
  support, `apiDownload`/`saveDownload` (token in a header, never in the URL)
  and a `conflict` error kind for 409.
- **Tests**: `tests/control_plane.rs` (8), `tests/remote_crawl.rs` (5),
  `tests/remote_export.rs` (6), `tests/remote_ai.rs` (7),
  `tests/artifact_download.rs` (7) — all against a real `harvest serve` and a
  local mock origin (no public network, no credentials); console suites
  `control-jobs.test.tsx` (16), `control-job-detail.test.tsx` (8),
  `artifacts.test.tsx` (12) against four fixtures recorded from a live control
  plane by `tools/record_control_fixtures.py`.
- **Docs**: `docs/api/24-control-plane-execution.md`,
  `docs/api/25-artifact-downloads.md`; three `docs/pages/` guides;
  `docs/modules/` entries for the five new modules.
- Counts after this wave: engine **655 tests** default (669 `--all-features`;
  460 unit via `cargo test --lib`); console **351 tests / 124 files**;
  verify-release VERIFIED (routes=26, modules=56, api-rows=34, tests=30).
  fmt/clippy `-D warnings` clean.

### Delivery & discovery wave (0.10.0 tree) — 787 → 825 tracked files
- **Crawl planning** (`src/crawl/mod.rs`, `src/crawl/discovery.rs`): a pure,
  bounded planner — multiple start URLs, depth/page/per-page caps, host scope,
  include/exclude (existing substring semantics), robots authority, content-type
  scope, sitemap + sitemap-index recursion, canonical/alternate/feed/document/
  media discovery, and metadata-only resource indexing. Pagination is declared
  (`rel=next/prev` or an operator selector) and never synthesized. Reuses
  `TargetCfg` via `CrawlPlan::from_target`; every refusal is counted.
- **Content/document/media layer** (`src/content.rs`, `src/document.rs`,
  `src/media.rs`): one MIME/charset classifier (headers authoritative, sniffing
  fills gaps, contradictions flagged, bodies over the cap refused before
  decoding), a typed document model with provenance and no embedded payloads,
  and resource discovery that records metadata only (no size ⇒ no invented
  checksum).
- **Optional AI assistance** (`src/ai.rs`, `src/ai/schema.rs`): provider trait,
  disabled provider, deterministic local provider, request/input budgets,
  timeout, bounded retries, field-name redaction, response validation and
  confidence routing (high ⇒ accept, medium ⇒ review, low ⇒ reject). Off by
  default; external providers need explicit `external_allowed`. AI failure
  never destroys deterministic data, and a validated value is never overwritten
  without `overwrite_validated`.
- **Export adapters** (`src/export/{xlsx,pdf,parquet,xml,bundle}.rs`): real
  XLSX (OOXML, typed cells, multi-sheet, bounded), real PDF (summary report,
  bounded sample), real Parquet (Parquet 1.0, PLAIN, RLE definition levels,
  Thrift-compact footer), deterministic XML (escaping + name safety), and a ZIP
  bundle with path-safety, `checksums.txt` and an in-archive
  `bundle-manifest.json`. All of them ride the **existing** export pipeline:
  same staging, same manifest, same atomic finalize, same `harvest verify`.
- **CLI**: `harvest export --format csv,jsonl,xlsx,pdf,parquet,xml,zip`
  (repeatable/comma-separated; default unchanged) and `--dataset <id>`
  (sanitized into `dataset-<id>.<ext>`). `harvest verify` reports
  `records_basis` so a delivery without JSONL is judged honestly.
- **Console** 20 → 23 routes, new `Delivery` nav group: **Export Center**
  (filter → preview → formats → prepare → verify → download), **Crawl Studio**
  (scope/discovery/content/limits/extraction with the planner's own ceilings and
  read-only engine limits from `GET /config`), **AI Assist** (provider status,
  consent blocking, `AI SUGGESTION` vs `VALIDATED`, confidence bands).
  Components: `ExportFormatPicker`, `DataPreviewTable`, `AiSuggestionCard`,
  `CrawlScopePanel`. No new API endpoints: generation stays with the CLI.
- **Docs**: `export-formats.md`, `ai-assistance.md`, `crawl-discovery.md`; three
  `docs/pages/` guides; `docs/modules/` regenerated (ai, content, crawl,
  document, media, export); README/web-client/testing/codebase-map/
  capability-matrix synced to measured numbers.
- **Tests**: `tests/crawl_discovery.rs` (15), `tests/ai_pipeline.rs` (21, local
  deterministic providers only), `tests/export_formats.rs` (14, incl. tamper and
  determinism), `tests/content_integrity.rs` (14); console suites
  `export-center.test.tsx` (15), `ai-assist.test.tsx` (13),
  `crawl-studio.test.tsx` (12). Smoke chunk gate 20 → 23.
- Counts after this wave: engine **517 tests** default (531 `--all-features`;
  356 unit via `cargo test --lib`); console **315 tests / 121 files**;
  `make links` 278/0; verify-release VERIFIED (routes=23, modules=51,
  api-rows=25, tests=25). fmt/clippy `-D warnings` clean.
- Behaviour change worth calling out: every delivery now also contains
  `checksums.txt` (sha256 per payload file, `sha256sum -c` compatible), listed
  in the manifest like any other payload file. The ZIP bundle is reported by the
  CLI and described by its own in-archive manifest; `harvest verify` lists it as
  an extra by design.

### Operational wave (0.10.0 tree) — 740 → 787 tracked files
- **New library modules** (each with inline unit suites; wired into
  `src/lib.rs`): `health` (lane classification + `lane_checks` shared with
  the `harvest health` offline branch), `cache` (LRU+TTL, injectable clock),
  `stream` (bounded pub/observe with drop accounting), `policy_engine`
  (pure gate-order decision function), `repository` (bounded JSONL read
  windows: 4 MiB / 512 rows), `search` (deterministic query layer over the
  history/event files — library-only, no new HTTP surface), `artifact`
  (manifest loading + seven-state delivery classification), `api_models`
  (shared Page/SearchResource/error vocabulary), `runtime_metrics`
  (fixed-key metrics block with an honest `complete` flag).
- **Runtime lifecycle**: `LifecycleState` (starting/running/degraded/
  draining/stopped; stopped is final), CAS-degraded latch, drain marker that
  survives a 0-ms uptime, bounded shutdown-reason vocab; `lifecycle_json()`
  kept separate from the fixture-pinned `/runtime` snapshot.
- **Integration suites** (`tests/` 16 → 21): `runtime_health` (7),
  `search_contract` (9), `artifact_integrity` (9), `policy_regression` (4),
  `final_release` — full mock-origin pipeline: engine run → quarantine +
  history + audit → export → verify → artifact classify → search, incl.
  one-byte tamper.
- **Console** 15 → 20 routes: Command Center, Run Detail (`/run?run=`),
  Event Stream (`/events` + JsonInspector that masks before display AND
  copy), Delivery Verify (browser digest check; CLI stays the authority),
  Config Diff (read-only, canonicalized). Shared modules `lib/dataFormatters`
  (re-exports `format.ts`), `lib/redaction` (deep-mask, copies only),
  `patterns/ActivityTimeline` (60-cap over the existing `Timeline`).
  New page suites `command-center.test.tsx` + `final-pages.test.tsx`
  (16 checks; console totals now 275 tests / 118 files). Smoke chunk gate
  raised 15 → 20 lazy route chunks.
- **Docs**: `search-operations.md` (bounds + determinism contract),
  `artifact-integrity.md` (state table + trust model), `policy-model.md`
  (gate order + safe defaults), `health-and-lifecycle.md`,
  `bounded-primitives.md`; five `docs/pages/` guides;
  README/web-README/web-client/testing/codebase-map/capability-matrix/
  release-status synced to measured numbers; `docs/modules/` index + nine
  module surfaces generated.
- **Tooling**: `tools/verify-release.py` — packaging gate for version
  coherence, route/module/API-manifest/test-list parity, example JSON,
  artifact-free `cargo package --list`, and no machine-specific paths.
- Counts after this wave: engine **397 tests** default (411 `--all-features`;
  300 unit via `cargo test --lib`); console **275 tests / 118 files**; smoke
  12/12; buyer E2E 18/18. fmt/clippy `-D warnings` clean.


### Buyer-first polish wave (0.10.0 tree) — 712 → 740 tracked files
- **API Explorer** page + `lib/apiRoutes/` manifest: 25-row single-source map
  of the admin API (23 GET + 2 POST), auth/params/doc columns verified against
  `src/admin.rs`; pinned by `tests/route_contract.rs` (new) and
  `web/src/test/contract/api-explorer.test.ts` (new) in both directions.
- **Overview HealthSummary** (System/Execution/Network/Data/Storage/Security)
  built only from `/stats`, `/runtime`, `/storage`, connection state — dashes
  before data, never placeholder zeros.
- **First-run WelcomePanel** (live values + copyable demo command; no fake
  run button — there is no run POST) and **optional ProductTour** (15 real
  routes, non-modal, dismissal persisted under `harvest.console.tour`;
  localStorage allow-list test extended accordingly).
- **StudioIndex** in Config Studio (37 controls computed from `CONTROLS`) and
  Target Builder step badges + live generated-config card.
- **Buyer E2E** `web/e2e/buyer.mjs` (18 checks, self-spawns demo site +
  engine + preview + CLI export/verify; wired into npm scripts, Makefile, and
  the CI release job); smoke gate now expects 15 lazy route chunks.
- Docs: `docs/buyer-demo.md` (A–AC), `docs/buyer-quickstart.md`,
  `docs/release-status.md`, `docs/capability-matrix.md`, `docs/codebase-map.md`,
  `docs/pages/api.md`, `web/README.md`; README gains the Mermaid pipeline
  diagram, CLI↔Web terminology table, and a measured bundle-size section.
  `tools/check_links.py` green (214 links).
- Packaging: explicit `[package] include` in Cargo.toml (review with
  `cargo package --list`) + `tests/repo_integrity.rs` guards it.
- Counts after this wave: engine 327 tests / 19 suites (330 all-features);
  console 259 tests / 116 files; smoke 12/12; buyer E2E 18/18; links 214/0.  No
  status badges anywhere — run the gates above yourself.


### Tests — per-unit coverage for every surface (575 → 712 tracked files)
- `web/src/test/components/`: 35 component contract tests covering every
  `ui/` primitive, every `patterns/` unit, and the icon registry (62 tests).
- `web/src/test/contract/`: 21 endpoint contract tests over the captured
  fixtures — envelope shapes, vocabularies, and corpus rules like "no version
  field is ever exposed" (29 tests).
- `tests/examples.rs`: every committed example config must parse through the
  real `Config` loader; production example pinned to fail loudly on unexpanded
  `${VAR}` secrets.
- Per-unit tests for every remaining surface: 19 files for Overlays/Table/
  AppShell (modal traps, DataTable sort/density/paging, TopBar/Sidebar/
  StatusBar through real providers) and 5 hook suites. Console suite now
  242 tests / 110 files — all green with the fixes below.

### Fixed
- `usePolled`: the idle-state effect overwrote `offline` with `live` whenever
  a poll failed with a network error — the connection dot never went red.
  Found by the new hook unit test.
- `isEditable` returned `undefined` (not `false`) for non-editable elements
  under engines where `isContentEditable` is absent; now strictly boolean.
- `docs/api.md`: 24 links to the `docs/api/` split files were left relative
  to the old single-file layout and 404'd; `tools/check_links.py` now gates
  every relative markdown link (`make links`).

### Docs
- `docs/pages/`: one feature doc per console page (14 + index) — routes,
  shortcuts, endpoints, and poll intervals measured from `ROUTES.ts` and the
  feature sources.
- `docs/modules/`: generated public-API index for all 38 Rust modules.

### Deploy
- `ops/docker-compose.yml`: static `console` service (nginx, read-only,
  `cap_drop: ALL`) proxying `/api/` to the engine; `web/Dockerfile` gained
  `ARG VITE_API_BASE` so the image is origin-agnostic.

Format follows Keep a Changelog; versions follow Semantic Versioning. The
project's public surface (CLI verbs, config fields, admin API, file
formats) is the compatibility contract; entries below describe actual
committed changes — dates are the repo's git dates, and history starts at
v0.7 (pre-v0.7 development predates this repository).

### Earlier in the same unreleased wave

#### Refactor — repo-wide modularization (168 → 575 tracked files)
- `web/src`: every monolith split into one file per unit behind a directory
  barrel — components/ui, patterns, Overlays, AppShell, Table, Icons (one
  file per glyph), every feature page (drawers/sections as siblings),
  `lib/types`, `lib/mock`, `lib/format`, hooks, router/routes, and the
  config schema (per-section control registries). Public import paths and
  names are byte-for-byte identical; tsc/eslint/vitest consumers unchanged.
- Styles: `app.css` → 14 ordered layers under `web/src/styles/app/`
  (`@import` chain); the design-system test resolves the chain.
- Tests: vitest suites split per describe block (30 files, 89 tests); Rust
  unit tests extracted from inline blocks into `src/<module>/tests.rs`
  file-pairs for all 38 modules (331 tests green).
- Docs: `docs/api.md` → 24 per-section references under `docs/api/`;
  `docs/adr/` with 14 decision records; `docs/testing.md`; `CONTRIBUTING.md`.
- Deploy: `web/Dockerfile` + nginx example, `deploy/{systemd,caddy,k8s}/`,
  root `Makefile` (the exact CI gate set), editorconfig/gitattributes/
  nvmrc/toolchain pins.
- Fixed stale counts (docs said 63 vitest; schema comment said 29 controls
  for 37 controls); vite `chunkFileNames` keeps per-route chunk names so the
  smoke test still asserts one chunk per route (12/12 green).

#### Added
- Web console (`web/`): React 18 + TypeScript + Vite operations UI for the
  admin API — 14 lazy-loaded pages covering stats/overview, run history
  (server-side window + drawer), scheduler jobs (with real retry/terminate
  ops), queue, workers, proxy health (id/region only — URLs never rendered),
  live activity via `since_seq` delta polling, pipeline chain read-out,
  quality funnel, storage lanes, alerts with double-guarded
  `POST /alerts/ack`, audit/ops, diagnostics with copy-report masking, and
  a draft-only config studio + per-target builder bound to the real
  `TargetCfg`/`TransformCfg` surface (37 controls, 23 ops). 89 vitest
  tests mount pages against fixtures captured from a running engine. No
  backend routes were added or invented; the admin API contract is
  unchanged.

#### hardening (web) — buyer-demo & resilience pass
- `lib/api.ts`: transport timeout (15 s / 4 s probes), one-shot retry for
  idempotent GETs only, single-flight dedup, normalized `ApiError`
  (timeout/aborted/slowdown kinds; 400→payload), `probeHealth()`, runtime
  `setApiBase` — token remains memory-only
- `usePolled`: 429 → one-interval backoff (verified on a live clock)
- first-run `ConnectScreen` (endpoint + bearer fields, live health probe,
  no fabricated backend version); loud `DEMO MODE` banner component
- `useShortcuts`: `g o/r/j/w/p/q/c` navigation with input+modifier guards
- persisted prefs: `harvest.console.nav|cadence`; Diagnostics cadence
  control; storage allow-list asserted by tests
- `vite preview` now proxies /api (parity with dev); `npm run smoke`:
  12-check real-dist e2e against a live harvest serve
- tests 63 → 89 (api/resilience/shortcuts + connect/demo/copy UX)

## [0.10.0] - 2026-09-20

### Fixed
- Release-readiness audit (buyer-facing pass over docs, help text, and
  formatting — no behavior changes): `harvest queue --help` no longer shows
  the "built without redis" stub on feature-enabled binaries; `docs/api.md`
  corrected the `POST /ops/note` body cap to the actual 4 KiB (`MAX_BODY`,
  verified live); README checkpoint section now states the measured
  crash-boundary contract (lost in-flight page vs. one post-save duplicate
  line, bounded by `ckpt_every_secs`) instead of an unconditional
  no-duplicates claim; full-tree `cargo fmt` so the CI fmt gate is green; three `uninlined_format_args`
  hits surfaced by the current stable clippy (two in `#[cfg(test)]` lib code, one
  in `tests/export_parts.rs`) fixed so `cargo clippy --all-targets --all-features
  -- -D warnings` — the exact CI gate — is green there too.

### Added
- Typed identity layer (`ids`): `TargetId`/`ProxyId`/`WorkerId`/`RunName`
  boundary-validated slugs (a-z 0-9 . _ -, ≤ 64 B, no dot-lead, no `..`,
  no trailing dot — ids become path components and metric labels),
  `RecordId` strict 64-hex sha256 fingerprints, `LeaseId` generated
  `<worker>-s<seq>-<8hex>` (parse-validates its own shape). Config
  validation now refuses malformed *or duplicated* target/proxy ids;
  checkpoints parse fingerprints through the type (undecodable entries
  drop at load instead of poisoning the dedupe set).
- Resource budgets (`budget` + `max_inflight_per_proxy` /
  `max_records_per_run`, both `0` = unlimited and byte-identical to prior
  behavior): the proxy pool refuses holds past the per-egress cap as a
  retryable transport failure — the existing backoff lane, never a new
  failure mode — and the engine stops taking records at the cap *after*
  dedupe+schema, then drains gracefully: one `budget_exhausted` event,
  exit 0, checkpoints and audit intact. Denials count in
  `harvest_budget_denied_total{scope="proxy"|"records"}` (JSON lockstep
  key `budget_denied_total`).
- Config schema versioning (`migrate`): `config_version` stamps configs
  (current 2; absence means v1). `harvest config migrate [--out F]
  [--dry-run] [--json]` rewrites a file through the migration table
  without expanding `${ENV}` — migration never copies secrets to disk.
  The v1→v2 step is stamp-only and says so in its own warning text.
  Unknown top-level keys fail at load with a one-edit-distance
  did-you-mean; `_`-prefixed keys are documented comments (ignored by
  loader, validator, and the key audit — `_note` in the examples is now
  contractual); newer-than-build files refuse to load rather than
  silently downgrade.
- Load-time warnings + production audit: migration notes surface as
  `warning:` lines in `harvest validate`, and as `_warnings` in
  `harvest config inspect` / `GET /config`. `production_audit()` — five
  soft checks (robots off, >5 req/s pacing, retries ≥ 8, concurrency
  > 64, >64 MiB responses) — feeds the same two surfaces (`_production`)
  plus `_backends` (which storages the binary was compiled with). The
  shipped default config audits clean, pinned by test; warnings never
  move an exit code.
- Delivery integrity (`export`, new `harvest verify`): `--max-part-bytes`
  chunks parts at record boundaries (a part is whole records; csv parts
  each carry their header; one oversized record becomes its own part);
  finalize is atomic (payload parts are staged, moved into place, and
  `manifest.json` lands last — a directory with a manifest is a complete
  export by construction); `--resume` after a crash validates every
  previous part by sha256 against a progress sidecar plus an
  input/options fingerprint before keeping it — valid parts are never
  rewritten (inode stability is asserted, not assumed).
  `export::verify()` / `harvest verify --dir D [--key F] [--json]`
  checks presence, bytes, sha256, line counts, the record total,
  unexpected files, and the detached manifest signature (exit 1 on any
  FAIL). `harvest config explain` now carries a `config_version` row;
  `--split-target` and the signing lane are unchanged (the part cap is a
  CLI/export option, not a config field).
- Dist worker hardening: the same per-egress in-flight cap applies in
  every shard; running jobs heartbeat a lease (TTL 30 s, refreshed at
  TTL/3) whose value is the holder's `LeaseId`, `sweep_inflight` is now
  lease-aware (live work is skipped, not double-processed; returns
  `{moved, held_by_lease}`), `--prefix` is validated as a redis key
  token before any I/O, queue-protocol failures are survivable
  (exponential backoff on pop, `queue_errors` counted, ack/dlq failures
  logged not fatal — the multiplexed connection reconnects underneath;
  60 consecutive pop failures is a hard stop), and `harvest worker
  --name` (slug; default `w-<pid>`) labels leases and logs.
- Metrics: `harvest_target_records_written{target="…"}` (the label set
  is bounded by the config; rendered only once non-empty) and
  `harvest_robots_cache_hits` (distributed politeness-cache answers).
- Testing & tooling: `tests/budgets.rs` (cap drains exact; contention
  observability with delayed responses), `tests/export_parts.rs`
  (chunking math, crash→resume, tamper + signature verdicts),
  `tests/properties.rs` (seeded-PRNG sweeps: id deformations, budget
  accounting under random interleaving, chunked ≡ single-file exports
  at arbitrary caps, migration fixed-point and fail-closure),
  `tests/faults.rs` (real engine runs over fault-injected origins);
  devtools mocks gained deterministic `Route::flaky/Reset/hang` with
  per-route hit counts; live-redis tests cover lease-sheltered sweeps;
  `cargo bench` — a zero-dependency ns/op table, machine-relative,
  recorded in `docs/development.md`.
- Extraction pipeline layer (two modules, every knob default-off):
  `transform` (per-field compiled op chains — text hygiene, currency/number
  parsing with documented thousands-separator semantics and exact numeric
  pass-through, five date formats → RFC3339/unix via the shared civil-date
  algorithm, unit normalization with KB/KiB distinct, to_bool vocabularies,
  split/join/list ops, quote-aware `strip_tags` with entity decoding, and
  `mask_pii` redaction gated by Luhn/octet checks; chains are atomic per
  field — a failure ships the raw value, never a half-transformed one),
  `schema` (declared per-target field types with `off`/`coerce`/`strict`
  enforcement: coercion counts `harvest_schema_drifts`, strict violations
  append the record + reasons to a JSONL quarantine lane and skip the sink
  while the job still succeeds and discovery continues). New `harvest
  schema --config … [--target id]` prints derived draft-07 contracts
  (declared types or transform-inferred) for CI. Config validation
  compiles transform regexes and schema shape up front. Distributed
  workers apply the identical pipeline. Metrics `records_quarantined` /
  `schema_drifts`; event kind `record_quarantined`; `Summary.quarantined` /
  `.schema_drifts` / per-target `quarantined`; dashboard key list extended;
  example `examples/config.pipeline.json`.
- Engine layer (nine modules, all wired into the crawl loop):
  `job` (identity, enforced lifecycle state machine, deadlines, retry
  metadata), `sched` (queue with fifo/priority/fair modes, bounded depth,
  per-target inflight gates, pop-time expiry, stats), `supervisor`
  (worker registry + health states, restart policy with backoff,
  cancel-token drain, bounded `TaskSet`), `exec` (request plan/context/
  metadata with correlation ids), `retry` (the delay curve factored out
  byte-identically, reason taxonomy, per-minute budget, deterministic
  test mode), `breaker` (per-host transport circuit with half-open probe),
  `accounting` (run counters, failure ring, retry history, throughput),
  `events` (bounded ring bus with drop accounting and rejoin-on-gap),
  `runtime` (single snapshot view owning them all).
- New config knobs, all defaulting to previous behavior: `events_path`
  (structured JSONL event log), `retry_deterministic`, `scheduling.*`
  (mode, max_queue_depth, per_target_inflight, worker_restarts,
  job_max_age_secs, retry_budget_per_min), `transport_breaker.*`
  (min_consecutive, cooldown_secs, recovery_successes). All validated
  at config time with footgun ranges documented.
- Admin API: `GET /runtime` (engine snapshot: accounting, queue, workers,
  open breakers, event-bus health) and `GET /events?limit=` (ring tail) —
  token-gated like every non-`/health` route; the 404 help line and docs
  list them.
- Metrics: five new counters (`jobs_completed`, `jobs_failed`,
  `queue_rejected`, `breaker_rejected`, `retry_budget_denied`) in
  `/stats` and the Prometheus render.
- Redis worker (`dist`): queue-side job expiry (`expires_at` from
  `job_max_age_secs`), per-process transport breaker with requeue-or-dead
  semantics, and the shared `execute` envelope so events/metrics flow in
  distributed mode too.
- Tests: 71 new unit tests across the nine modules (all state machines,
  bounds, and formulas pinned) + `tests/engine_layer.rs` integration suite
  (8 scenarios against in-process mocks).
- `docs/`: architecture, API/contracts, configuration, deployment,
  troubleshooting and development guides; `examples/`: basic, production
  and distributed configs, request snippets, and real output samples signed
  end-to-end during live smokes (regenerated this pass — records, signed
  audit.json(.sig), signed manifest(.sig) from a fresh demo-mock run, each
  verified by `harvest audit --md`, `attest --verify`, and manifest
  sha256-chain recomputation).
- `LICENSE`: MIT, matching the `Cargo.toml` declaration (file was missing).

### Fixed
- CLI broken-pipe behavior: Rust ignores SIGPIPE, so piping any
  stdout-printing command into an early-exiting consumer
  (`harvest history --n 20 | head -1`) panicked in `println!`
  ("failed printing to stdout: Broken pipe"). `main()` now restores the
  default disposition — the process dies by SIGPIPE (status 141) exactly
  like other unix tools; pinned by `tests/cli.rs` (direct `libc` dep,
  already transitively present).

### Changed
- `Config::load` runs strict-before-lenient: expand → key-audit → migrate
  → deserialize → validate, so a typo'd key produces the did-you-mean,
  not a serde error three layers down.
- Slug validation now rejects `..` inside and `.` at the end of any id
  (previously only the bare `..`); ids feed filesystem paths.
- README quick start: step 5/6 now use paths the preceding steps actually
  produce (`out/demo.jsonl`, `out/demo-runs.jsonl`, the bundled audit
  sample); scheduler note says 10s (the demo config's value), not 10 min.
- `ops/docker-compose.yml`: harvest service now declares
  `stop_signal: SIGINT` (+120 s grace) — the engine drains on SIGINT only;
  a `docker stop` under the default SIGTERM no longer looks like a clean
  shutdown it never performed.

## [0.9.0] - 2026-09-17

### Added
- HMAC-SHA256 attestation: `audit.json.sig` / `manifest.json.sig` written
  when `attest_key_path` is set; `harvest attest --gen-key/--verify` for
  key lifecycle and closed-loop verification (verify fails closed).
- Security & production hardening pass:
  - fail-closed `validate()` catalog (bounds on concurrency/timeouts/
    retries/body caps, admin bind/token rules, proxy shape, sink/selector
    sanity) — every refusal names the rule and remedy;
  - whole-value `"${VAR}"` env injection for secrets; missing variable is a
    named startup error; secret-bearing URLs redacted in all logs/errors;
  - response-size caps enforced twice (declared `Content-Length` and
    streamed) with a `Permanent` error class that is never retried;
    redirect count capped at 5;
  - admin API: bearer-token auth on every route except `GET /health`
    (constant body), timing-safe compare, `nosniff` + `no-store`,
    401 bodies carry zero request detail; `metrics_addr:""` disables it;
  - poison-tolerant locks (a panicking task can't brick a long run);
  - Docker image: non-root uid, `read_only`-friendly layout, volume chown,
    bash `/dev/tcp` HEALTHCHECK (no curl in runtime); compose: loopback
    port binds, `cap_drop: [ALL]`, `no-new-privileges`, Prometheus bearer
    passthrough via `$HARVEST_ADMIN_TOKEN`;
  - `SECURITY.md` (threat model + disclosure), `tests/security.rs`, and CI
    jobs incl. per-feature compile matrix and release smoke.

## [0.8.0] - 2026-09-17

### Added
- Per-host politeness audit: `audit_path` records every request class,
  wait, refusal and robots event per host; `harvest audit --file … --md`
  renders the receipt a crawl leaves for site owners and customers.

## [0.7.0] - 2026-09-17

### Added
- `harvest export`: deterministic client-delivery folder (wide CSV with
  dot-flattened fields + canonical JSONL + sha256 manifest), `--history`
  embeds the policy hash per run, `--split-target`, skipping is always
  reported (`skipped_lines`), never silent.

## Pre-0.7 (no tagged entries)

Core surface this changelog assumes as background, all present in 0.9: the
politeness engine (robots, per-domain token bucket + block breaker),
HTML/JSON extraction, sinks (jsonl/csv/sqlite/postgres/dataset/http/null),
checkpoint/resume with policy-drift refusal, DLQ + `--retry-dlq`,
conditional-GET revalidation, sitemap seeding, proxy pool with health
scoring, Redis distributed mode (`push/worker/queue`), scheduler
(`serve`) + run history, admin dashboard/metrics, SLO alert webhooks, and
the build/CI/test harness.
