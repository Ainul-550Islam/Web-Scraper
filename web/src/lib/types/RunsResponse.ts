// RunsResponse — split from types.ts; barrel re-exports keep every public path & name

import type { RunInfo } from "./RunInfo";
import type { Paged } from "./Paged";

export interface RunsResponse {
  page: Paged<RunInfo>;
  window: number;
  window_note: string;
}
