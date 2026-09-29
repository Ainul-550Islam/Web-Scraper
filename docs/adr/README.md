# Architecture decision records

One record per consequential, hard-to-reverse choice. Numbered, immutable once accepted;
supersede by adding a new record that references the old one.

| # | Decision | Status |
|---|----------|--------|
| 0001 | Use hash routing for the console SPA | Accepted |
| 0002 | Poll with per-page intervals instead of SSE or WebSocket | Accepted |
| 0003 | Retry GETs exactly once; never retry POSTs | Accepted |
| 0004 | Contract fixtures are captured from a running engine | Accepted |
| 0005 | Demo data is compiled out of production builds | Accepted |
| 0006 | The admin bearer token lives only in module memory | Accepted |
| 0007 | Config studio edits a draft; writes are explicit and single-shot | Accepted |
| 0008 | Secrets are masked engine-side AND redacted in the console | Accepted |
| 0009 | Never display version metadata the API does not expose | Accepted |
| 0010 | Split monolith modules into one file per unit behind barrels | Accepted |
| 0011 | Rust unit tests live in src/<module>/tests.rs file-modules | Accepted |
| 0012 | Unit tests in jsdom/vitest; a separate smoke test hits the real dist | Accepted |
| 0013 | Keyboard navigation uses a g-prefix chord with a 1.5s window | Accepted |
| 0014 | Each page is its own lazily-loaded chunk | Accepted |
| 0015 | Console static hosting reaches the engine through a same-origin /api proxy | Accepted |
