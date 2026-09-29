// RUNS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { TS } from "./TS";


export const RUNS: T.RunsResponse = {
  page: {
    items: [
      { config_sha256: "e9b681df59f8f520150dcbf1c004043152621b06c55a98ccd4889f0389202335", duration_ms: 1170, error: null, index: 3, quarantined: 0, run: "demo-run", status: "ok", ts: TS + 30, written: 5 },
      { config_sha256: "e9b681df59f8f520150dcbf1c004043152621b06c55a98ccd4889f0389202335", duration_ms: 1182, error: null, index: 2, quarantined: 0, run: "demo-run", status: "ok", ts: TS + 20, written: 5 },
      { config_sha256: "e9b681df59f8f520150dcbf1c004043152621b06c55a98ccd4889f0389202335", duration_ms: 1204, error: null, index: 1, quarantined: 0, run: "demo-run", status: "ok", ts: TS + 10, written: 5 },
      { config_sha256: "e9b681df59f8f520150dcbf1c004043152621b06c55a98ccd4889f0389202335", duration_ms: 1168, error: null, index: 0, quarantined: 0, run: "demo-run", status: "ok", ts: TS, written: 5 },
    ],
    limit: 50, offset: 0, total: 4,
  },
  window: 4,
  window_note: "listings scan the newest 512 history lines; `total` is within that window",
};
