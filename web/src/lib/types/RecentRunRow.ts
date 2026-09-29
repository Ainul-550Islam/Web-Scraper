// RecentRunRow — split from types.ts; barrel re-exports keep every public path & name

import type { RunInfo } from "./RunInfo";


/* recent: raw history rows (superset of RunInfo: per-target summary) */
export interface RecentRunRow extends RunInfo {
  respect_robots: boolean;
  summary: {
    blocked: number;
    ckpt_writes: number;
    deduped: number;
    duration_ms: number;
    pages_fetched: number;
    per_target: Record<
      string,
      {
        blocked: number;
        failed: number;
        quarantined: number;
        revalidated: number;
        written: number;
      }
    >;
    retries: number;
    robots_denied: number;
    written: number;
    [k: string]: unknown;
  };
}
