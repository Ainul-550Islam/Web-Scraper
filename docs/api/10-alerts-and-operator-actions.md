### Alerts and operator actions

`GET /alerts` returns the dedup ledger (`items[]` with `event`, `state`,
`severity`, `fires`, `notified`, `suppressed`, last detail — all bounded at
128 entries) plus the `fired`/`notified` metric totals and the configured
cooldown/severity floor.

`POST /alerts/ack?event=NAME|*` acknowledges alerts (the `*` form acks all
fire-and-forget), `POST /ops/note` records an operator note (JSON body
`{"note": "…"}`) — sanitized and truncated to 200 chars before it enters
the ops ring. Both write an audit line; neither executes anything.

```
GET /ops     → operator-action ring (serve_start, run_start/run_end,
               notes, acks, config.inspect…) newest-first, capped 256
GET /audit   → audit records: {ts, actor, action, outcome, detail, corr}
               — metadata + correlation id only, never request bodies
GET /config  → the live config through the same redaction as
               `harvest config inspect`: secret fields become
               "<redacted>", URL credentials are stripped, and
               "_sources" reports provenance per key:
               "explicit" | "default" | "env:VAR_NAME".
               Also carried: "_warnings" (config migration notes from
               load time), "_production" (soft production-audit findings),
               "_backends" (sqlite/postgres/redis availability of the
               running binary) — inserted after redaction, so they are
               never scrubbed
```
