// QUEUE — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { RUNTIME } from "./RUNTIME";

export const QUEUE: T.QueueHealth = {
  state: "healthy",
  stats: RUNTIME.queue,
  avg_queue_wait_ms: 18.5,
};
