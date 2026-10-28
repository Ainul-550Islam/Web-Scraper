# Codebase map

Orient-new-engineer tour of the tree. Sizes/counts in this file are computed,
never asserted: regenerate with the commands shown.

## Rust engine (src/)

One crate, `harvest`: library + thin binary. Each domain module is
`src/<name>.rs` (types + logic) with unit corpus in `src/<name>/tests.rs`,
wired via `#[cfg(test)] mod tests;` in the parent file. `src/dist.rs` +
`src/dist/tests.rs` hold distribution/queue-adjacent unit tests.

Top-down pipeline order (what happens to one page):

```
config (src/config.rs) ─ validate at startup, policy sha
  → scheduler (src/sched.rs) — every_secs loop, runs cap, history
  → engine (src/engine.rs) — run orchestration, budgets wiring, SIGINT/SIGTERM drain
  → runner (src/runner.rs) — crawl frontier, dlq replay, robots gate call
  → fetcher (src/fetcher.rs) — reqwest client, reval (src/reval.rs),
    retry/backoff (src/retry.rs), breaker (src/breaker.rs),
    proxy (src/proxy.rs) egress, politeness permits (policy)
  → extract (src/extract.rs) — jsonpath/css selectors
  → transform (src/transform.rs) — typed op chain
  → schema (src/schema.rs) — enforce + drift events
  → sink (src/sink.rs) — fingerprint dedupe (ids.rs), quarantine lane,
    jsonl | sqlite feature | postgres via pgsink.rs (redis exec in exec.rs)
  → ckpt (src/ckpt.rs), audit (src/audit.rs), events (src/events.rs),
    metrics (src/metrics.rs), alerts (src/alerts.rs), ops (src/ops.rs),
    accounting (src/accounting.rs)
admin plane: src/admin.rs (router: 28 GET + 6 POST) + src/admin/ submodules,
  helpers in src/api.rs; signing in src/sign.rs; export in src/export.rs.
CLI: src/main.rs (clap: run, serve, history, validate, schema, push,
  worker, queue, status, health, jobs, runs, workers, proxies, metrics,
  datasets, config, audit, export, verify, attest, inspect, explain,
  migrate).
Support: testutil (devtools feature only), error.rs, lib.rs, version.rs
  (build.rs embeds git describe), migrate.rs, exec.rs (worker loop),
  supervisor.rs (pool), export parts logic.
```

Regenerate a module listing: `ls src/*.rs | wc -l`. Cross-module
integration tests live in `tests/` (30 files: ai_pipeline, artifact_download,
artifact_integrity, budgets, cli, cli_ops, content_integrity, control_plane,
crawl_discovery, dist, e2e, engine_layer, examples, export_formats,
export_parts, faults, final_release, ops_api, pg, policy_regression,
properties, remote_ai, remote_crawl, remote_export, repo_integrity,
route_contract, runtime_health, search_contract, security,
workflow_devtools).

## React console (web/src/)

Strict TS, no UI kit dependency. Layering rule (enforced by import
boundaries in review): `app/` → `features/` → `components/` → `hooks/` →
`lib/`.

```
app/        routes (ROUTES.ts: 26 entries, metadata incl. lazy loaders),
            router (hash router), App shell composition, providers
components/ ui/ (28 component files incl. DataTable, Segmented, Modal, Drawer),
            patterns/ (FilterBar, FormSection, MetricCard…, plus the control
            plane's JobStateBadge, CommandComposer and ArtifactList), AppShell/
            (TopBar, StatusBar, Sidebar, palette, tour, welcome),
            Charts/ (Sparkline, FunnelStrip…), Icons/ (svg registry)
features/   one folder per page, plus pages/ (ExportCenter, CrawlStudio,
            AiAssist, ControlJobs, ControlJobDetail, Artifacts):
            Overview(+HealthSummary,IntelligenceRow),
            Runs(+RunDrawer,runTimeline), Jobs, QueuePage, Workers, Proxies,
            Activity(events), Pipeline, Quality, Storage, Alerts, Audit,
            Diagnostics, ConfigStudio(+StudioIndex,Control,SENSITIVE),
            TargetBuilder, Api/ApiExplorer(+lib/apiRoutes manifest)
hooks/      usePolled (bounded self-rescheduling interval), useTheme,
            useToasts, usePersistentState, useLocalStorageSafe, …
lib/        api.ts (typed client, 405-safe, idempotency-key + artifact
            download support), connection (provider: health/
            config/alerts/latency), configSchema (CONTROLS registry — 37 controls,
            computed from sections/general…alerts at build-of-module time), apiRoutes/ (manifest single-source), types/
            (every endpoint has a hand-typed response), format, mock/
            (dev-only fixtures), ux/ (a11y helpers)
test/       harness.tsx (fixture-backed fetch stub), fixtures/ (24 captured
            responses, byte-preserved, ADR 0004 — four of them recorded from a
            live control plane by tools/record_control_fixtures.py), contract/, pages/,
            components/, hooks/, ux/ (01-theme…07-firstrun)
```

## Tooling & packaging

```
build.rs            embeds version info (git describe, fallback "dev")
tools/check_links.py  relative-markdown-link checker (`make links`)
Makefile            fmt/lint/test/build/links/smoke/buyer-e2e/all
demo/server.py      deterministic mock site for examples/config.basic.json
examples/*.json     validated by tests/examples.rs (5 configs)
docs/               api/ (26) · pages/ (27, including the index) · adr/ · this map · buyer docs
docker/             compose stack for console+engine demo (static-validated)
```

Verify any count claimed above:
`ls web/src/components/ui/*.tsx | wc -l`, `grep -c 'path: "' web/src/app/routes/ROUTES.ts`,
`ls docs/api/*.md | wc -l`, `cargo test 2>&1 | tail` etc. — this file is a
map, and the tests are the contract.
