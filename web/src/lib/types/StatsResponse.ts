// StatsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { MetricsCounters } from "./MetricsCounters";
import type { ProxySnapshot } from "./ProxySnapshot";


export interface StatsResponse {
  running: boolean;
  metrics: MetricsCounters;
  proxies: ProxySnapshot[];
}
