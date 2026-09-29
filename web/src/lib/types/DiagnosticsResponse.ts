// DiagnosticsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { DiagStatus } from "./DiagStatus";
import type { DiagComponent } from "./DiagComponent";

export interface DiagnosticsResponse {
  components: DiagComponent[];
  generated_ts: number;
  overall: DiagStatus;
}
