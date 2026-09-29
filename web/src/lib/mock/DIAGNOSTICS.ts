// DIAGNOSTICS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { TS } from "./TS";


export const DIAGNOSTICS: T.DiagnosticsResponse = {
  components: [
    { name: "admin_uptime", status: "healthy", detail: { ms: 61220 } },
    { name: "alerts", status: "disabled", detail: { entries: 0 } },
    { name: "breakers", status: "healthy", detail: { nonclosed: 0 } },
    { name: "config", status: "healthy", detail: { targets: 2 } },
    { name: "events_log", status: "disabled", detail: { bytes: 0, configured: false } },
    { name: "history", status: "healthy", detail: { bytes: 577, exists: true } },
    { name: "output", status: "healthy", detail: { exists: true, kind: "jsonl/csv file" } },
    { name: "proxies", status: "healthy", detail: { total: 1, healthy: 1, degraded: 0, down: 0, unprobed: 0 } },
    { name: "queue", status: "healthy", detail: { depth: 0, rejected: 0 } },
    { name: "runtime", status: "healthy", detail: { running: true } },
    { name: "workers", status: "healthy", detail: { busy: 0, deaths: 0 } },
  ],
  generated_ts: TS + 32,
  overall: "healthy",
};
