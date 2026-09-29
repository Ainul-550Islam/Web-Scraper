# Audit Explorer

| | |
|---|---|
| Route | `/audit` |
| Nav group | Data & Guardrails |
| Shortcut | — |
| Endpoints | `/audit?limit=200`, `/ops?limit=200` |
| Polling | 8–12 s |

Sealed politeness journal and the operator feed side by side, newest first.
`limit` is honored by the engine (fixtures `audit_limit_4`/`events_limit_4`
pin the envelope). Rows are read-only by design: the console never writes to
the audit lane — that is a CLI/`/ops` action.

- **See also:** [api/19 audit format](../api/19-politeness-audit-format-auditjson.md), [api/21 operations CLI](../api/21-operations-cli.md)
