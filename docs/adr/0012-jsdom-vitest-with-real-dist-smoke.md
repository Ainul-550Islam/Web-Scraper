# Unit tests in jsdom/vitest; a separate smoke test hits the real dist

Status: **Accepted**

## Context & decision

Playwright would bolt a browser download onto every CI job; deterministic DOM-level checks suffice at this surface size. 30 vitest files mount real pages against captured fixtures, and `npm run smoke` serves the actual `vite build` output and runs 12 checks against a live `harvest serve` — the unit layer stays fast while the shipped bundle itself is verified end-to-end.
