// JOBS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";


export const JOBS: T.JobsResponse = {
  job: {
    jobs: [
      { target: "demo-catalog", written: 4, blocked: 0, failed: 0, quarantined: 0, revalidated: 0 },
      { target: "demo-news", written: 1, blocked: 0, failed: 0, quarantined: 0, revalidated: 0 },
    ],
    run: "demo-run", status: "running", uptime_ms: 61220,
  },
  note: "per-target tallies come from the most recent persisted run",
};
