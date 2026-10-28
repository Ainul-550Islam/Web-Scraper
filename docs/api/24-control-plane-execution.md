# Control-plane execution (`/control/*`)

The admin API's execution surface. When `admin_token` is configured, every
route here requires its bearer token; `/health` and `/live` are the only
routes exempt from that token. With no token configured, the API's normal
loopback-only policy applies and a non-loopback bind is rejected. The
crawl/export/AI submission routes accept an optional `Idempotency-Key`
(recommended; the console sends one). Cancellation is
state-aware but is not idempotency-keyed: cancelling a terminal job returns
`409`.

The control plane does not contain a second engine. A command is validated,
turned into one bounded job, and handed to machinery that already exists:

| Operation | Delegates to | Produces |
|---|---|---|
| `crawl` | `harvest::engine` (the same run loop the scheduler uses) | a run in the history ledger + the configured output file registered as an artifact |
| `export` | `harvest::export_job` over `harvest::export` | a delivery directory (`records.csv`, `records.jsonl`, `manifest.json`, `checksums.txt`, plus the formats asked for) |
| `ai` | `harvest::ai_job` over `harvest::ai` | metadata only — counts, bands, suggestions, and the reason when the round could not help |

## The command

```
POST /control/crawl
POST /control/export
POST /control/ai
authorization: Bearer <admin_token>
idempotency-key: <8..128 chars of [A-Za-z0-9_-]>   (optional, recommended)
content-type: application/json

{ "op": "crawl" | "export" | "ai", ..., "limits": { ... } }
```

* The body is capped at 64 KiB (`COMMAND_BODY_LIMIT`); larger is `413`.
* `op` must match the route: posting an `export` command to `/control/crawl` is
  `400`, not a silent reinterpretation.
* Unknown keys in `limits` are refused — a typo must not become a default.

### `limits` (all optional; ceilings from `src/control.rs`)

| Field | Default | Ceiling | Refused when |
|---|---|---|---|
| `max_pages` | 100 | 1,000,000 | 0 or > ceiling |
| `max_depth` | 1 | 20 | > ceiling |
| `max_records` | 100,000 | 5,000,000 | 0 or > ceiling |
| `max_export_bytes` | 268,435,456 | 1,073,741,824 | 0 or > ceiling |
| `max_ai_input_bytes` | 1,048,576 | 4,194,304 | 0 or > ceiling |
| `timeout_secs` | 900 | 3,600 | 0 or > ceiling |

Before a job starts, the executor **clamps these down** to whatever the
attached configuration allows (`CommandLimits::clamp_to_config`), so a command
can never out-run the config it was submitted against.

### `crawl`

```json
{ "op": "crawl", "targets": ["catalogue"], "limits": { "max_pages": 50 } }
```

* `targets` names ids **declared in the attached config**. If the config
  declares more than one target and the command names none, the answer is
  `400` — "all of them" is ambiguous and ambiguity is not a default.
* Ids are charset-checked (`[A-Za-z0-9_-]`, ≤ 64 chars).

### `export`

```json
{ "op": "export", "formats": ["csv","jsonl","xlsx","pdf","parquet","xml","zip"],
  "dataset": "client-drop", "bundle": false, "verify": true, "max_records": 10000,
  "limits": { "max_export_bytes": 268435456 } }
```

* `formats` accepts the seven `harvest::export::ExportFormat` values
  (case- and whitespace-insensitive) and nothing else.
* `zip` in the list (or `bundle: true`) means "bundle the rest".
* `max_records`, when supplied, must be at least 1; `0` is rejected. Omit it or
  send `null` to use `limits.max_records` (default 100,000); the effective
  record count is clamped to that limit.
* `verify: true` runs `harvest verify` over the delivery before the job reports
  success; a delivery that does not verify is a **failed** job, not a warning.
* Over the `max_export_bytes` cap the job fails with the byte count in the
  reason. Nothing partial is registered.

### `ai`

```json
{ "op": "ai", "task": "classify", "provider": "deterministic-local",
  "confidence": 0.9, "external_allowed": false,
  "source": { "kind": "run_output", "record_limit": 100, "field_limit": 32 } }
```

* Provider table: **`deterministic-local`** and **`disabled`**. Anything else is
  `400`, and the refusal names what is available. Consent
  (`external_allowed`) is recorded in the audit line; it cannot summon a
  provider this build does not compile in.
* Tasks: `extract`, `classify`, `normalize`, `schema-suggest`, `label`.
* `confidence` is in `[0, 1]`. Below it, a suggestion is reported as `review`
  and never applied.
* The job applies the outcome to a scratch record: **the run output is never
  rewritten**, whatever the provider says.
* An AI job registers no artifacts — there is nothing to download.

## Jobs

```
GET  /control/jobs?state=&op=&limit=&offset=
GET  /control/jobs/:id
POST /control/jobs/:id/cancel
```

States are `queued → running → completed | failed | cancelled`, and only the
server performs a transition (`ExecState::can_transition`); illegal moves are
refused, not absorbed. A job carries `created_ts + limits.timeout_secs` as a
deadline, and `get`/`list` re-check it, so a task that dies without reporting
cannot leave a job `running` forever.

| Situation | Answer |
|---|---|
| no token / wrong token | `401`, `WWW-Authenticate: Bearer`, body identical to every other 401 |
| malformed or out-of-bounds command | `400` with the ceiling in the message |
| body over 64 KiB | `413` |
| unknown job id, or an id that is not `job-<n>` | `404` |
| 8 concurrent jobs, or 64 in flight | `503` (clean refusal — nothing is queued silently) |
| cancel on a terminal job | `409` ("already completed") |
| idempotency key reused with a different body | `409` |
| idempotency key reused with the same body | `200` + the original job, `replayed: true` |

Responses carry **no filesystem paths, no tokens, no payloads**: counters,
limits, artifact *names*, hashes, content types, states, and an `audit_ref`
that points at the server's own ledger line.

## Audit

Every submission, start, finish, cancel and refusal writes one bounded record
(`actor`, `action`, `result`, `resource`, `corr`, and a small `detail` for AI
jobs naming the provider and whether external processing was consented to).
The console keeps no copy of its own.
