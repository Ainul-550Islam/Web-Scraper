# Each page is its own lazily-loaded chunk

Status: **Accepted**

## Context & decision

`routes.ts` declares pages as `() => import(…)`, so the shell (index 43.0 kB / 14.8 kB gzip, vendor 140.7 / 45.1 — measured on the current HEAD) paints first and each page streams in behind a skeleton. The build gate asserts one emitted chunk per route; dynamic imports are retried nowhere — a failed chunk surfaces as an ordinary error state (§0003's philosophy: no hidden retries).
