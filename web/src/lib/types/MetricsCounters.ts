// MetricsCounters — split from types.ts; barrel re-exports keep every public path & name



export interface MetricsCounters {
  alerts_fired: number;
  alerts_notified: number;
  api_request_ms_total: number;
  api_requests_2xx: number;
  api_requests_4xx: number;
  api_requests_5xx: number;
  api_requests_total: number;
  backoff_sleep_ms_total: number;
  breaker_rejected: number;
  budget_denied_proxy: number;
  budget_denied_records: number;
  budget_denied_total: number;
  checkpoints_written: number;
  domains_abandoned: number;
  jobs_completed: number;
  jobs_failed: number;
  pages_fetched: number;
  pipeline_failures: number;
  politeness_permits: number;
  politeness_sleeps: number;
  queue_rejected: number;
  records_deduped: number;
  records_extracted: number;
  records_quarantined: number;
  records_written: number;
  request_ms_total: number;
  requests_blocked: number;
  requests_ok: number;
  requests_retry: number;
  requests_revalidated: number;
  requests_total: number;
  retry_budget_denied: number;
  robots_cache_hits: number;
  robots_denials: number;
  robots_fetches: number;
  schema_drifts: number;
  storage_write_ms_total: number;
  storage_writes: number;
  target_records_written: Record<string, number>;
}
