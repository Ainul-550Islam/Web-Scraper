// EVENT_KINDS — split from types.ts; barrel re-exports keep every public path & name



/* ── events feed ──────────────────────────────────────────────── */
export const EVENT_KINDS = [
  "breaker_closed",
  "breaker_opened",
  "budget_exhausted",
  "extraction_completed",
  "job_completed",
  "job_expired",
  "job_failed",
  "job_started",
  "job_submitted",
  "operator_action",
  "proxy_failure",
  "proxy_selected",
  "queue_overflow",
  "record_quarantined",
  "request_finished",
  "request_started",
  "retry_scheduled",
  "shutdown_completed",
  "shutdown_started",
  "sink_write_completed",
] as const;
