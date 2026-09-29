// KIND_TONE — split from Activity.tsx; barrel re-exports keep every public path & name


export const KIND_TONE: Record<string, string> = {
  request_finished: "info", request_started: "idle", proxy_failure: "err",
  retry_scheduled: "warn", record_quarantined: "warn", job_failed: "err",
  job_completed: "ok", queue_overflow: "err", breaker_opened: "err",
  breaker_closed: "ok", sink_write_completed: "ok", budget_exhausted: "warn",
};
