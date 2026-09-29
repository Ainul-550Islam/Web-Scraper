# Alerts

| | |
|---|---|
| Route | `/alerts` |
| Nav group | Data & Guardrails |
| Shortcut | — |
| Endpoints | `/alerts` |
| Polling | 7 s |

SLO + lifecycle alerts with cooldown state, `fired`/`notified` counters, and
the nav badge that keeps the count while you are away. `min_severity` filters
server-side; the page just renders the returned items — empty is a real state.

- **See also:** [api/10 alerts & operator actions](../api/10-alerts-and-operator-actions.md)
