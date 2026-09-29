// AlertsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { AlertItem } from "./AlertItem";

export interface AlertsResponse {
  fired: number;
  notified: number;
  cooldown_secs: number;
  min_severity: string;
  items: AlertItem[];
}
