# Config studio edits a draft; writes are explicit and single-shot

Status: **Accepted**

## Context & decision

GET /config seeds the draft and every control mutates the draft only. Save issues exactly one write through the documented path and re-seeds from the server response. Bounds validation mirrors the engine locally (`configSchema.ts` cross-field rules), but the server remains authoritative — the UI never pretends a partial write succeeded.
