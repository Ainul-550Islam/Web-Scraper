// FailuresResponse — split from types.ts; barrel re-exports keep every public path & name

import type { RecentFailure } from "./RecentFailure";


/* ── failures / retries ───────────────────────────────────────── */
export interface FailuresResponse {
  state: "available" | "unavailable";
  detail?: string;
  total: number;
  items: RecentFailure[];
}
