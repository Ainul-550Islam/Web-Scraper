# Each page is its own lazily-loaded chunk

Status: **Accepted**

## Context & decision

`web/src/app/routes/PAGE_LOADERS.ts` declares pages as dynamic imports, so the shell paints first and each page streams in behind a skeleton. The reviewed 2026-10-05 production build emitted 26 route chunks; its entry script was 56.49 kB raw / 19.15 kB gzip, vendor 140.80 / 45.27, and stylesheet 37.19 / 8.11. These measurements are descriptive, not size gates. The build gate asserts one emitted chunk per route; dynamic imports are retried nowhere — a failed chunk surfaces as an ordinary error state (§0003's philosophy: no hidden retries).
