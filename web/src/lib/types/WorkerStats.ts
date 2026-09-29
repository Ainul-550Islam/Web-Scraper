// WorkerStats — split from types.ts; barrel re-exports keep every public path & name

import type { WorkerInfo } from "./WorkerInfo";

export interface WorkerStats {
  busy: number;
  deaths: number;
  draining: boolean;
  registered: number;
  total_restarts: number;
  utilization: number;
  workers: WorkerInfo[];
}
