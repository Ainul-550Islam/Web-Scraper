// WorkerHealth — split from types.ts; barrel re-exports keep every public path & name

import type { WorkerStats } from "./WorkerStats";
import type { ComponentStatus } from "./ComponentStatus";


export interface WorkerHealth {
  state: ComponentStatus;
  snapshot: WorkerStats | null;
}
