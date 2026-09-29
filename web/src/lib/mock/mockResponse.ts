// mockResponse — split from mock.ts; barrel re-exports keep every public path & name

import type * as T from "../types";
import { MOCK_ENABLED } from "./MOCK_ENABLED";
import { STATS } from "./STATS";
import { RUNTIME } from "./RUNTIME";
import { JOBS } from "./JOBS";
import { RUNS } from "./RUNS";
import { RECENT } from "./RECENT";
import { EVENTS } from "./EVENTS";
import { WORKERS } from "./WORKERS";
import { QUEUE } from "./QUEUE";
import { PROXIES } from "./PROXIES";
import { PIPELINE } from "./PIPELINE";
import { STORAGE } from "./STORAGE";
import { DIAGNOSTICS } from "./DIAGNOSTICS";
import { ALERTS } from "./ALERTS";
import { OPS } from "./OPS";
import { AUDIT } from "./AUDIT";
import { CONFIG } from "./CONFIG";

export function mockResponse(path: string): unknown | null {
  if (!MOCK_ENABLED) return null;
  const route = path.split("?")[0] as string;
  switch (route) {
    case "/health":
      return { status: "ok", running: true } satisfies T.HealthResponse;
    case "/live":
      return { live: true };
    case "/stats":
      return STATS;
    case "/runtime":
      return RUNTIME;
    case "/jobs":
      return JOBS;
    case "/runs":
      return RUNS;
    case "/recent":
      return RECENT;
    case "/failures":
      return { state: "available", total: 0, items: [] };
    case "/retries":
      return { state: "available", accounting_retries: 0, requests_retry: 0, recent_delays_ms: [] };
    case "/workers":
      return WORKERS;
    case "/queue":
      return QUEUE;
    case "/proxies":
      return PROXIES;
    case "/pipeline":
      return PIPELINE;
    case "/storage":
      return STORAGE;
    case "/diagnostics":
      return DIAGNOSTICS;
    case "/alerts":
      return ALERTS;
    case "/ops":
      return OPS;
    case "/audit":
      return AUDIT;
    case "/config":
      return CONFIG;
    case "/events":
      return EVENTS;
    case "/policy":
      return {
        robots_txt: "enforced (RFC-9309-ish, cached, deny => skip URL)",
        pacing: "per-domain token bucket + min interval",
        refusals: "401/403/429/451 => Retry-After honored, exponential backoff, circuit breaker abandons domain",
        proxy_rotation: "health-based failover/geo only; never used to evade blocks",
        fingerprint_spoofing: "not implemented, by design",
        user_agent: "honest bot identity enforced by config validation",
      };
    default:
      return { error: { code: "not_found", message: "route not found (mock)" }, request_id: "req-mock" };
  }
}
