// WORKERS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { RUNTIME } from "./RUNTIME";


export const WORKERS: T.WorkerHealth = {
  state: "healthy",
  snapshot: RUNTIME.workers,
};
