# Diagnostics

| | |
|---|---|
| Route | `/diagnostics` |
| Nav group | Data & Guardrails |
| Shortcut | — |
| Endpoints | `/diagnostics` |
| Polling | 10 s |

Component-by-component verdicts (`{detail, name, status}` with status ∈
healthy | degraded | disabled | error) plus one overall verdict, all copyable
via `CopyButton` — the payload is already the exact JSON the CLI prints, so
bug reports survive copy-paste unchanged.

- **See also:** [api/09 internal health checks](../api/09-internal-health-checks-distinct-from-hea.md)
