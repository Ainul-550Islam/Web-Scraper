// EventsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { EngineEvent } from "./EngineEvent";

export interface EventsResponse {
  limit: number;
  newest_first: boolean;
  ring_len: number;
  dropped: number;
  last_seq: number;
  events: EngineEvent[];
}
