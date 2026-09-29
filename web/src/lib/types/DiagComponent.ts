// DiagComponent — split from types.ts; barrel re-exports keep every public path & name

import type { DiagStatus } from "./DiagStatus";

export interface DiagComponent {
  name: string;
  status: DiagStatus;
  detail: Record<string, unknown>;
}
