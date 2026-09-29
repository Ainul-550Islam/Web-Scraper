// RECENT — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { RUNS } from "./RUNS";


export const RECENT: T.RecentRunRow[] = RUNS.page.items.map((r) => ({
  ...r,
  respect_robots: true,
  summary: {
    blocked: 0, ckpt_writes: 0, deduped: 0, duration_ms: r.duration_ms, pages_fetched: 5,
    per_target: {
      "demo-catalog": { blocked: 0, failed: 0, quarantined: 0, revalidated: 0, written: 4 },
      "demo-news": { blocked: 0, failed: 0, quarantined: 0, revalidated: 0, written: 1 },
    },
    retries: 0, robots_denied: 1, written: 5,
  },
}));
