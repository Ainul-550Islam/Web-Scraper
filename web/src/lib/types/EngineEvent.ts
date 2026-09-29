// EngineEvent — split from types.ts; barrel re-exports keep every public path & name

import type { EventKind } from "./EventKind";


export interface EngineEvent {
  seq: number;
  ts: number;
  kind: EventKind;
  job: number | null;
  detail: {
    url?: string;
    status?: number;
    bytes?: number;
    ms?: number;
    attempt?: number;
    target?: number;
    worker?: number;
    depth?: number;
    queue_wait_ms?: number;
    [k: string]: unknown;
  };
}
