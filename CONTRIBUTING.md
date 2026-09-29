# Contributing

Every change lands through the same gate set — locally, in CI, and in review.
There are no shortcuts below that don't also fail CI.

## Environment

- **Rust:** pinned channel via `rust-toolchain.toml` (stable + rustfmt + clippy).
- **Node:** 20 (`web/.nvmrc`; CI uses `node:20-alpine`).
- `cargo fetch` once, then `cd web && npm ci` (the lockfile is committed;
  `npm ci` is the only sanctioned install).

## The gates

From the repo root, `make` runs everything. Individually:

```sh
cargo fmt --all                                          # formatting is enforced
cargo clippy --all-targets --all-features -- -D warnings # zero warnings, zero exceptions
cargo test --all-features                                # 330 engine tests across 19 suites (327 default), incl. per-module src/<m>/tests.rs
cd web
npx tsc --noEmit                                         # strict project; no implicit any
npx eslint .                                             # 0 errors and 0 warnings
npx vitest run                                           # 259 console tests across 116 files
python3 tools/check_links.py .                             # every relative markdown link resolves
npm run build                                            # dist + one chunk per route asserted
npm run smoke                                            # real dist vs live `harvest serve` :9678
```

The smoke script boots the engine binary, serves the actual `dist/`, and runs
12 end-to-end checks against real HTTP — it is the only test that may touch
networked state, and it cleans the process group up on exit.

## House rules (all enforced, none aspirational)

1. **The admin API is the spec.** No route, field, or status code may be
   invented in client or docs; fixtures under `web/src/test/fixtures/` are
   captured from a running engine and regenerate-only (docs/adr/0004).
2. **One unit per file behind a barrel.** Components, icon glyphs, CSS
   layers, schema sections, and test suites each live in their own file; the
   public module path is a directory barrel (docs/adr/0010).
3. **Rust tests are file-modules.** `src/x.rs` pairs with `src/x/tests.rs`;
   never add an `x/mod.rs` alongside `x.rs` (E0761) (docs/adr/0011).
4. **Secrets never persist.** Bearer tokens stay in memory; exports are
   masked engine-side and redacted again in the console (docs/adr/0006, 0008).
5. **Numbers in docs are measured, never remembered.** Test counts and bundle
   sizes in CHANGELOG/testing docs get updated in the same commit that
   changes them.
6. **Consequential choices get an ADR** under `docs/adr/` — context plus
   decision, one page, immutable once accepted.

## Layout map

| Path | Contents |
|------|----------|
| `src/` | engine: `x.rs` modules with `src/x/tests.rs` unit-test pairs |
| `tests/` | CLI, budget, dist-integration suites (cargo test) |
| `web/src/components/` | ui kit, icons (per-glyph), shell, overlays — barrels at top |
| `web/src/features/<page>/` | one directory per page; sections/drawers as sibling units |
| `web/src/lib/` | api transport, types, mock dev-router, config schema (per-section registries) |
| `web/src/styles/app/` | design-system CSS layers, imported in order by `app.css` |
| `web/src/test/` | suites split per describe block; fixtures are captured JSON only |
| `docs/` | buyer docs + `api/` per-section API reference + `adr/` |
| `deploy/`, `ops/` | systemd, nginx, caddy, k8s, docker-compose, dashboards |
