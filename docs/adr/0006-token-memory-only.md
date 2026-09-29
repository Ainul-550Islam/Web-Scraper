# The admin bearer token lives only in module memory

Status: **Accepted**

## Context & decision

Persisting credentials in browser storage is off the table: `lib/api.ts` keeps `bearer` in a module closure and never writes it to any storage; only the API base URL (non-sensitive) persists. Consequence: a hard reload re-prompts via the ConnectScreen when the engine requires auth — accepted as the price of a zero-credential-at-rest browser surface.
