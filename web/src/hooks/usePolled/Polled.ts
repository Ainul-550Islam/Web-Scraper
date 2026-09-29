// Polled — split from usePolled.ts; barrel re-exports keep every public path & name

import type { ApiError } from "../../lib/api";
import type { PollState } from "./PollState";


export interface Polled<T> {
  data: T | null;
  error: ApiError | null;
  state: PollState;
  updatedAt: number | null;
  refresh: () => void;
  setPaused: (p: boolean) => void;
}
