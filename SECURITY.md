# Security Policy

## Supported versions

We harden the current `main` only; security-relevant fixes ship in the next
tagged release. There is no backport commitment yet (pre-1.0).

## Reporting a vulnerability

Email **security@example.com** (replace with the project's real address
before public release) with a plain description and reproduction steps.
Expect a triage response within 5 business days. Do not open a public issue
for anything that lets one party bypass another's access controls.

## What the engine enforces (and where the code proves it)

These are behavioral guarantees, each pinned by a test — not aspirations:

- **robots.txt is the default; refusals back off, never hammer.**
  401/403/429/451 honor `Retry-After`, exponential backoff, and a per-host
  circuit breaker (`src/policy.rs`, `tests/e2e.rs::refusals_are_honored_not_hammered`).
- **Admin API fails closed.** With `admin_token` set, every endpoint except
  the constant-content `/health` probe requires a timing-safe Bearer match;
  error responses never echo the path or any token material
  (`src/admin.rs` unit tests). Binding `metrics_addr` to a routable address
  *without* a token is a startup error, not a warning (`src/config.rs`).
- **Outbound bounds are enforced.** Response bodies are capped
  (`max_response_bytes`, default 16 MiB): declared length is checked at the
  header and unbounded chunked streams are aborted mid-body; both classify
  as *permanent* — no retry loop (`src/fetcher.rs`,
  `tests/security.rs::response_size_caps_reject_without_retry_or_oom`).
  Config validation also caps one response at 64 MiB and the nominal
  `cap × effective workers` in-flight response-body reservation at 1 GiB
  (effective workers are capped at 64). This is a body-buffer ceiling, not a
  total-process RSS guarantee; DOM parsing and other runtime memory are
  additional. Redirects stop after 5. Retries cap at 10 (validation) with a
  `max_single_wait_ms` sleep ceiling. Timeouts are mandatory (1..=600 s).
- **Configured paths cannot climb.** `..` in any output/history/checkpoint/
  audit path is rejected at validate time (`src/config.rs`).
- **Secrets are never in files by necessity.** Whole-value `"${VAR}"`
  references in the config JSON are expanded from the environment at load;
  an undefined variable aborts startup instead of being used literally.
  URLs carrying credentials (`proxy.url`, `redis_url`, alert webhooks) are
  redacted before they can reach any log line
  (`redact_credentials` / `redact_url_origin`, `src/config.rs`).
- **Attested evidence.** `audit.json` and export `manifest.json` can be
  HMAC-SHA256-signed with `harvest attest`; verification fails closed on any
  single-bit edit, and keys are generated mode 0600 with overwrite refused
  (`src/sign.rs`, `tests/security.rs::attest_key_lifecycle...`).
- **Proxy pool = reliability only.** Fixed-size from config (≤ 256),
  EWMA health failover, schemes restricted to http/https/socks5/socks5h.
  It exists for egress management, not for evading blocks: a refusal is
  recorded against the *host*, and rotating proxies do not reset a host
  breaker (`README.md`, "What this is / what it is deliberately not").

## What harvest deliberately is NOT

No fingerprint spoofing, no CAPTCHA or bot-detection bypass, no
credential-stuffing or account access, no ban-evasion. A config that sets a
browser-spoofing `user_agent` while `respect_robots` is on is rejected at
load. Features that would turn this into an evasion tool will be declined.

## Operational checklist for production

1. Set `admin_token` via the environment: `"admin_token": "${HARVEST_ADMIN_TOKEN}"`
   — generate with `head -c24 /dev/urandom | base64`. Keep it ≥ 12 chars.
2. Put DB/Redis/proxy credentials in `${VAR}` references too; never commit
   literal secrets. In `ops/docker-compose.yml` pass them via
   `environment:` / host env — the image carries none.
3. Use `rediss://` for remote Redis carrying AUTH, TLS-capable Postgres
   DSN params (`sslmode=require`), and http(s) proxy URLs with credentials
   only to your own proxy fleet.
4. `ops/docker-compose.yml` binds Prometheus/Grafana/admin to `127.0.0.1`
   by default. Prometheus and Grafana have their own auth stories — expose
   them behind a reverse proxy with TLS, never by deleting the prefix.
5. The Grafana admin password in compose is a DEV placeholder: override
   `GRAFANA_ADMIN_PASSWORD` before `docker compose up`.
6. The container runs as uid 10001, read-only rootfs, all capabilities
   dropped, `/var/lib/harvest` is the single writable volume.
7. Rotate `attest_key_path` keys by policy, not necessity: old receipts
   stay verifiable as long as the old key is retained (keep them archived
   read-only).

## Deeper documentation

Operational detail lives in
[`docs/deployment.md`](docs/deployment.md) (topologies, exposure, backups),
[`docs/api.md`](docs/api.md) (admin API contract incl. the auth exemption on
`/health`), [`docs/configuration.md`](docs/configuration.md) (every
validation rule) and [`docs/troubleshooting.md`](docs/troubleshooting.md)
(failure signatures for the guards above).
