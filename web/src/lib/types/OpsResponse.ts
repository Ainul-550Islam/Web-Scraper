// OpsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { OpsEntry } from "./OpsEntry";

export interface OpsResponse {
  items: OpsEntry[];
}
