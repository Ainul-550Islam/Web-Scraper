# Demo data is compiled out of production builds

Status: **Accepted**

## Context & decision

`MOCK_ENABLED` is `import.meta.env.DEV`, so a production `vite build` contains zero mock bytes: no demo rows, no fabricated numbers. The offline dev experience (fixture router in `lib/mock`) stays available for `npm run dev` and vitest. A visible DEMO MODE banner marks every mock-backed screen so nobody mistakes demo state for engine state.
