// OPS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { TS } from "./TS";


export const OPS: T.OpsResponse = {
  items: [
    { ts: TS + 30, kind: "run_start", detail: { every_secs: 10, run: "demo-run" } },
    { ts: TS + 29, kind: "run_end", detail: { status: "ok", duration_ms: 1182, written: 5, quarantined: 0, alerts_fired: 0 } },
    { ts: TS + 20, kind: "run_start", detail: { every_secs: 10, run: "demo-run" } },
    { ts: TS + 19, kind: "run_end", detail: { status: "ok", duration_ms: 1204, written: 5, quarantined: 0, alerts_fired: 0 } },
    { ts: TS - 1, kind: "serve_start", detail: { addr: "127.0.0.1:9678" } },
  ],
};
