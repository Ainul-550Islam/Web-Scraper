// ConnectionProvider — split from connection.tsx; barrel re-exports keep every public path & name

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { ApiError, apiGet, auth, invalidateCache, OPTS } from "../api";
import type { AlertsResponse, ConfigDocument, HealthResponse } from "../types";
import { usePolled } from "../../hooks/usePolled";
import { useToasts } from "../../hooks/useToasts";
import type { ConnectionState } from "./ConnectionState";
import { Ctx } from "./Ctx";
import type { Bundle } from "./Bundle";


export function ConnectionProvider({ children }: { children: ReactNode }) {
  const { push } = useToasts();
  const [tokenSet, setTokenSet] = useState(auth.has());
  // §32: cadence is a plain preference (like theme/nav/density) — safe to
  // persist; nothing credential-shaped ever hits localStorage.
  const [intervalSecs, setIntervalSecsRaw] = useState(() => {
    let v = 6;
    try {
      const saved = Number(localStorage.getItem("harvest.console.cadence"));
      if (Number.isFinite(saved) && saved >= 3 && saved <= 120) v = saved;
    } catch { /* storage optional */ }
    return v;
  });
  const setIntervalSecs = useCallback((n: number) => {
    if (!Number.isFinite(n)) return;
    const v = Math.min(120, Math.max(3, Math.round(n)));
    setIntervalSecsRaw(v);
    try {
      localStorage.setItem("harvest.console.cadence", String(v));
    } catch { /* storage optional */ }
  }, []);
  const wasDown = useRef<boolean | null>(null);
  const [latencyMs, setLatencyMs] = useState<number | null>(null);

  const fetcher = useCallback(async (signal: AbortSignal): Promise<Bundle> => {
    const t0 = performance.now();
    const health = await apiGet<HealthResponse>("/health", { signal, noCache: true });
    const ms = Math.round(performance.now() - t0);
    setLatencyMs(ms);
    const [config, alerts] = await Promise.all([
      apiGet<ConfigDocument>("/config", { signal }).catch((e: unknown) => {
        if (e instanceof ApiError && (e.kind === "unauthorized" || e.kind === "aborted")) throw e;
        return null;
      }),
      apiGet<AlertsResponse>("/alerts", { signal }).catch((e: unknown) => {
        if (e instanceof ApiError && (e.kind === "unauthorized" || e.kind === "aborted")) throw e;
        return null;
      }),
    ]);
    return { health, config, alerts };
  }, []);

  const polled = usePolled(fetcher, intervalSecs * 1000);

  // discrete connect/disconnect toasts only — polling itself stays silent
  useEffect(() => {
    const down = polled.error?.kind === "network" || polled.state === "offline";
    if (wasDown.current === null) {
      if (!down && polled.data) push("ok", "Connected to the admin API");
    } else if (down !== wasDown.current) {
      push(down ? "err" : "ok", down ? "Admin API unreachable" : "Connection restored");
    }
    wasDown.current = down;
  }, [polled.data, polled.error, polled.state, push]);

  const setToken = useCallback((t: string | null) => {
    auth.set(t);
    setTokenSet(auth.has());
    invalidateCache();
    polled.refresh();
    push(t ? "ok" : "info", t ? "Bearer token set (memory only)" : "Bearer token cleared");
  }, [push, polled]);

  const refreshAll = useCallback(() => {
    invalidateCache();
    polled.refresh();
  }, [polled]);

  const value: ConnectionState = {
    base: OPTS.BASE,
    health: polled.data?.health ?? null,
    config: polled.data?.config ?? null,
    alerts: polled.data?.alerts ?? null,
    latencyMs,
    state: polled.state,
    error: polled.error,
    tokenSet,
    setToken,
    refreshAll,
    updatedAt: polled.updatedAt,
    intervalSecs,
    setIntervalSecs,
  };
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}
