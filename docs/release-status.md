# Release status — 0.10.0

Plain-language state of this codebase for evaluation purposes. No badges, no
marketing numbers: everything here points at a command you can run.

## What ships

- Rust 2021 crate `harvest` (engine + CLI, single binary), version
  `0.10.0` in `Cargo.toml`/`Cargo.lock`, printed by `harvest --version`.
- React console in `web/` (Vite + TS, strict), served standalone via
  `npm run build && npm run preview` (or any static host); the binary also
  ships a tiny built-in status dashboard at `GET /` (single file, separate
  from the React console).
- Documentation set: README, `docs/api/` (26 files), `docs/pages/`
  (27 files incl. the index), `docs/modules/` (58 files), operational guides
  (search-operations, artifact-integrity, policy-model, health-and-lifecycle,
  bounded-primitives), ADRs, this status, capability matrix, codebase map.
- Deterministic loopback demo: `demo/server.py` + `examples/config.basic.json`.
- CI (GitHub Actions): fmt, clippy `-D`, full test matrix incl. Redis +
  Postgres services, feature-combo builds, web suite + build + smoke +
  buyer E2E, release build + CLI smoke incl. attestation fail-closed check.

## Verification status

| claim | how you confirm |
|---|---|
| engine tests | `cargo test` (default) + `cargo test --all-features` — suite counts in [testing.md](testing.md) |
| console unit/contract | `cd web && npm test` (jsdom, fixture-backed) |
| console static gates | `npm run typecheck && npm run lint` |
| built app + live API | `npm run build && npm run smoke` |
| full buyer flow | `npm run e2e:buyer` → `BUYER E2E: 18 passed, 0 failed` |
| docs integrity | `make links` → 0 broken links (tool: `tools/check_links.py`) |
| package contents | `cargo package --list` (include list is explicit in Cargo.toml) |

## Known limits (not gaps in docs — deliberate scope)

- Attestation is HMAC (shared-key) only; no asymmetric signing yet.
- The admin API is read-only except `/alerts/ack`, `/ops/note` and the four
  control-plane mutations (`POST /control/{crawl,export,ai}`,
  `POST /control/jobs/:id/cancel`). Everything else is 405 with an
  explanation, by design, enforced by route-contract tests. The control plane
  is fail-closed on the same bearer token and reuses the engine's own crawl,
  export and AI machinery — there is no second executor.
- Postgres/Redis paths need their servers + `--features postgres|redis`.
- No Kubernetes/operator tooling; single-binary scheduler or per-host
  `worker` processes only.
- `docker/` compose files are static-validated in CI context; building the
  image inside this sandbox was UNVERIFIED (no Docker daemon available).

## Release engineering hygiene

- `make all` = fmt → lint → test → build → links → smoke.
- No tracked build artifacts (`git ls-files` contains no dist/ or target/).
- Fixtures are captured responses only (ADR 0004) and byte-compared; the
  test suite never edits them.
- Secrets policy: token memory-only in the browser (ADR 0006); API echoes
  redact secrets (`admin_token: null`).
