// ProxyPoolResponse — split from Proxies.tsx; barrel re-exports keep every public path & name

import type { ProxyPoolEntry } from "./ProxyPoolEntry";

export interface ProxyPoolResponse {
  total: number;
  healthy: number;
  degraded: number;
  down: number;
  unprobed: number;
  inflight_denied: number;
  entries: ProxyPoolEntry[];
}
