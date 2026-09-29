// ProxyPoolEntry — split from Proxies.tsx; barrel re-exports keep every public path & name

import type { ProxySnapshot } from "../../../lib/types";

export interface ProxyPoolEntry extends ProxySnapshot {
  state: "healthy" | "degraded" | "down" | "unprobed" | string;
}
