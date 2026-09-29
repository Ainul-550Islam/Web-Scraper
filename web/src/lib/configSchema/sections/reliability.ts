// Reliability section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const RELIABILITY_CONTROLS: ControlDef[] = [
  // ── Reliability ──────────────────────────────────────────────
    { path: "max_retries", label: "Max retries", section: "Reliability", kind: "number", int: true, min: 0, max: 10,
      help: "Retries after the first attempt, for transient classes only." },
  { path: "retry_base_ms", label: "Retry backoff base", section: "Reliability", kind: "number", int: true, min: 0, unit: "ms",
      help: "Exponential base delay between attempts." },
  { path: "max_single_wait_ms", label: "Max single wait", section: "Reliability", kind: "number", int: true, min: 0, unit: "ms",
      help: "Clamps any single backoff sleep." },
  { path: "retry_deterministic", label: "Deterministic retries", section: "Reliability", kind: "toggle",
      help: "Disables jitter — retry delays become reproducible for tests." },
  { path: "block_backoff_secs", label: "Block backoff", section: "Reliability", kind: "number", int: true, min: 0, unit: "s",
      help: "401/403/429/451 refusal handling (Retry-After honored)." },
  { path: "block_breaker_count", label: "Circuit-breaker threshold", section: "Reliability", kind: "number", int: true, min: 0,
      help: "Consecutive refusals that open the per-domain breaker." },
  { path: "robots_ttl_secs", label: "robots cache TTL", section: "Reliability", kind: "number", int: true, min: 0, unit: "s",
      help: "How long parsed robots.txt results are cached per host." },
  { path: "max_pages", label: "Max pages (global)", section: "Reliability", kind: "number", int: true, min: 1,
      help: "Per-target crawl budget default." },
  { path: "max_records_per_run", label: "Records hard cap", section: "Reliability", kind: "number", int: true, min: 0,
      help: "Write-side cap; the run drains gracefully at the limit. 0 = unlimited." },
];
