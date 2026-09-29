// RuntimeSnapshot — split from types.ts; barrel re-exports keep every public path & name

import type { MetricsCounters } from "./MetricsCounters";
import type { RuntimeAccounting } from "./RuntimeAccounting";
import type { RuntimeEventBus } from "./RuntimeEventBus";
import type { QueueStats } from "./QueueStats";
import type { WorkerStats } from "./WorkerStats";

export interface RuntimeSnapshot {
  accounting: RuntimeAccounting;
  breakers: Record<string, unknown>;
  engine: { run: string; status: string; uptime_ms: number };
  events: RuntimeEventBus;
  metrics: MetricsCounters;
  queue: QueueStats;
  workers: WorkerStats;
}
