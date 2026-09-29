// JobsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { JobTally } from "./JobTally";

export interface JobsResponse {
  job: {
    jobs: JobTally[];
    run: string;
    status: string;
    uptime_ms: number;
  };
  note?: string;
}
