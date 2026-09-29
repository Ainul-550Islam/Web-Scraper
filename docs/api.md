# API reference

Two surfaces are documented here because both are contracts you build on:
the **admin HTTP API** (live operations) and the **file/record formats**
(batch consumers, exports, evidence). The API is an observation surface:
everything is a read except two explicit operator actions
(`POST /alerts/ack`, `POST /ops/note`); there is no remote job submission.
Unknown paths return 404, a known path with the wrong method returns 405.

## Reference sections

- [Admin HTTP API](api/00-admin-http-api.md)
- [`GET /health` — liveness (auth-exempt by design)](api/01-get-health-liveness-authexempt-by-design.md)
- [`GET /metrics` — Prometheus text exposition](api/02-get-metrics-prometheus-text-exposition.md)
- [`GET /stats` — live JSON snapshot](api/03-get-stats-live-json-snapshot.md)
- [`GET /runtime` — engine runtime snapshot](api/04-get-runtime-engine-runtime-snapshot.md)
- [`GET /events` — structured event tail](api/05-get-events-structured-event-tail.md)
- [Quarantine lane (`targets[].schema.quarantine_path`)](api/06-quarantine-lane-targetsschemaquarantinep.md)
- [`GET /recent` — newest run-history entries](api/07-get-recent-newest-runhistory-entries.md)
- [Operational views (jobs, runs, pipeline)](api/08-operational-views-jobs-runs-pipeline.md)
- [Internal health checks (distinct from `/health`)](api/09-internal-health-checks-distinct-from-hea.md)
- [Alerts and operator actions](api/10-alerts-and-operator-actions.md)
- [`GET /policy` — static self-description](api/11-get-policy-static-selfdescription.md)
- [`GET /` — status dashboard](api/12-get-status-dashboard.md)
- [Everything else](api/13-everything-else.md)
- [Record format (JSONL sink, and the input contract of `export`)](api/14-record-format-jsonl-sink-and-the-input-c.md)
- [Run-history format](api/15-runhistory-format.md)
- [Dead-letter format](api/16-deadletter-format.md)
- [Checkpoint format](api/17-checkpoint-format.md)
- [Revalidation store format](api/18-revalidation-store-format.md)
- [Politeness audit format (`audit.json`)](api/19-politeness-audit-format-auditjson.md)
- [Export manifest (`manifest.json`)](api/20-export-manifest-manifestjson.md)
- [Operations CLI](api/21-operations-cli.md)
- [CLI exit codes](api/22-cli-exit-codes.md)
- [`harvest fetch <url>`](api/23-harvest-fetch-url.md)

Every section lives in `docs/api/`, split one file per contract area; this index keeps
the intro verbatim so existing citations to `docs/api.md` still land on real content.
