// RUNTIME — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { TS } from "./TS";
import { STATS } from "./STATS";


export const RUNTIME: T.RuntimeSnapshot = {
  accounting: {
    avg_queue_wait_ms: 18.5, bytes_received: 2190, elapsed_ms: 61220, jobs_active: 0,
    jobs_cancelled: 0, jobs_completed: 6, jobs_expired: 0, jobs_failed: 0,
    jobs_per_sec: 0.098, jobs_submitted: 6, recent_failures: [], recent_retry_delays_ms: [],
    requests: 12, retries: 0, started_ts: TS,
  },
  breakers: {},
  engine: { run: "demo-run", status: "running", uptime_ms: 61220 },
  events: { dropped: 0, last_seq: 48, published: 48, ring_len: 48 },
  metrics: STATS.metrics,
  queue: {
    depth: 0, expired: 0, inflight: 0, max_depth: 0, mode: "fifo",
    per_target_active: [], per_target_depths: [], popped: 6, pushed: 6, rejected: 0,
  },
  workers: {
    busy: 0, deaths: 0, draining: false, registered: 2, total_restarts: 0, utilization: 0.0,
    workers: [
      { id: 0, idle_secs: 4, jobs_done: 4, jobs_failed: 0, restarts: 0, state: "running" },
      { id: 1, idle_secs: 4, jobs_done: 2, jobs_failed: 0, restarts: 0, state: "running" },
    ],
  },
};
