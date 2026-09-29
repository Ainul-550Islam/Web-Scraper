// STORAGE — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";


export const STORAGE: T.StorageResponse = {
  audit: { kind: "audit json", path: "", bytes: null, configured: false, exists: false },
  checkpoint: { kind: "checkpoint json", path: "", bytes: null, configured: false, exists: false },
  events: { kind: "event log jsonl", path: "", bytes: null, configured: false, exists: false },
  history: { kind: "history jsonl", path: "…/out/demo-runs.jsonl", bytes: 577, configured: true, exists: true },
  output: { kind: "jsonl/csv file", path: "…/out/demo.jsonl", bytes: 2080, configured: true, exists: true },
  quarantine: { kind: "quarantine jsonl", path: "", bytes: null, configured: false, exists: false },
  revalidation: { kind: "revalidation store", path: "", bytes: null, configured: false, exists: false },
};
