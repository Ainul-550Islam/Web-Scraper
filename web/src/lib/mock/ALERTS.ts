// ALERTS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";


export const ALERTS: T.AlertsResponse = { fired: 0, notified: 0, cooldown_secs: 0, min_severity: "", items: [] };
