# Configuration Studio

| | |
|---|---|
| Route | `/config` |
| Nav group | Console |
| Shortcut | g c |
| Endpoints | config document + dry-run draft (see `lib/api` config surface) |
| Polling | none — explicit refresh only |

Edits the REAL config document: 37 controls, each seeded from the server's
actual value (verified in [controls.test.tsx](../../web/src/test/controls.test.tsx)).
Everything is a client-side draft until you apply it; the token lives in memory
only ([ADR 0006](../adr/0006-token-memory-only.md)) and masked exports replace
secrets with `null` (see [contract/config.test.ts](../../web/src/test/contract/config.test.ts)).
Section anchors (`sec-<id>`) let deep links land on one group.

- **See also:** [configuration.md](../configuration.md), [api/13 everything else](../api/13-everything-else.md)
