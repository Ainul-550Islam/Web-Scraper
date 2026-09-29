// RuntimeAccounting — split from types.ts; barrel re-exports keep every public path & name

import type { RecentFailure } from "./RecentFailure";


/* ── runtime (live-run only; `error: "no run active"` otherwise) */
export interface RuntimeAccounting {
  avg_queue_wait_ms: number;
  bytes_received: number;
  elapsed_ms: number;
  jobs_active: number;
  jobs_cancelled: number;
  jobs_completed: number;
  jobs_expired: number;
  jobs_failed: number;
  jobs_per_sec: number;
  jobs_submitted: number;
  recent_failures: RecentFailure[];
  recent_retry_delays_ms: number[];
  requests: number;
  retries: number;
  started_ts: number;
}
