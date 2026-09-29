// AuditResponse — split from types.ts; barrel re-exports keep every public path & name

import type { AuditEntry } from "./AuditEntry";

export interface AuditResponse {
  items: AuditEntry[];
  note?: string;
}
