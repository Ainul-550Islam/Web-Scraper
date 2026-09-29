// EVENTS — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { TS } from "./TS";


export const EVENTS: T.EventsResponse = {
  limit: 64, newest_first: true, ring_len: 48, dropped: 0, last_seq: 48,
  events: [
    { seq: 48, ts: TS + 31, kind: "sink_write_completed", job: 6, detail: { target: 1, records: 1 } },
    { seq: 47, ts: TS + 31, kind: "request_finished", job: 6, detail: { url: "http://127.0.0.1:8099/news.html", status: 200, bytes: 321, ms: 4, attempt: 1 } },
    { seq: 46, ts: TS + 30, kind: "request_started", job: 6, detail: { url: "http://127.0.0.1:8099/news.html", attempt: 1 } },
    { seq: 45, ts: TS + 30, kind: "job_started", job: 6, detail: { worker: 1, target: 1, url: "http://127.0.0.1:8099/news.html", queue_wait_ms: 12, depth: 0 } },
    { seq: 44, ts: TS + 30, kind: "proxy_selected", job: 6, detail: { proxy: "direct", url: "http://127.0.0.1:8099/news.html" } },
    { seq: 43, ts: TS + 29, kind: "job_submitted", job: 6, detail: { target: 1, url: "http://127.0.0.1:8099/news.html" } },
    { seq: 42, ts: TS + 29, kind: "extraction_completed", job: 5, detail: { target: 0, fields: 4, url: "http://127.0.0.1:8099/product/3" } },
  ],
};
