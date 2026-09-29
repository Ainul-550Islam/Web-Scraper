# Never display version metadata the API does not expose

Status: **Accepted**

## Context & decision

The admin API (`/health`, `/runtime`) returns no version field, so the connect screen deliberately shows no version banner. Fabricating one would be the first lie in the product. If the engine ever adds a version field, `types.ts` plus the captured fixtures force the UI change through the same contract tripwire.
