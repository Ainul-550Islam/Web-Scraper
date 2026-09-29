# Request Activity

| | |
|---|---|
| Route | `/activity` |
| Nav group | Network & Flow |
| Shortcut | — |
| Endpoints | `/failures?limit=25`, `/retries` (plus the shared ring-backed `/events` feed) |
| Polling | 4–6 s |

Live per-request ring: status, size, and retry delays for the newest samples.
`dropped` from the ring envelope is displayed, never hidden — if the engine
truncated, the console says so. Retries use the same delay-bucket view as Jobs.

- **See also:** [api/05 events](../api/05-get-events-structured-event-tail.md)
