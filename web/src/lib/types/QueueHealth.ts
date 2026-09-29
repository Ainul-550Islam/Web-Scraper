// QueueHealth — split from types.ts; barrel re-exports keep every public path & name

import type { QueueStats } from "./QueueStats";
import type { ComponentStatus } from "./ComponentStatus";

export interface QueueHealth {
  state: ComponentStatus;
  stats: QueueStats | null;
  avg_queue_wait_ms: number;
}
