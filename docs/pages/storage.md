# Storage Observatory

| | |
|---|---|
| Route | `/storage` |
| Nav group | Data & Guardrails |
| Shortcut | — |
| Endpoints | `/storage` |
| Polling | 10 s |

Every configured lane (output, quarantine, audit, checkpoint, events,
history, revalidation) with `configured / exists / bytes / kind / path` per the
[storage contract](../../web/src/test/contract/storage.test.ts). `bytes: null`
means "asked, not answered" and renders as an em dash — never `0`, which would
lie about empty-but-present files.

- **See also:** [api/17 checkpoint format](../api/17-checkpoint-format.md), [api/19 audit format](../api/19-politeness-audit-format-auditjson.md)
