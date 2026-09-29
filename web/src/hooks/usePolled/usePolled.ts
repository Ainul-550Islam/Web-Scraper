// usePolled — split from usePolled.ts; barrel re-exports keep every public path & name

import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError } from "../../lib/api";
import type { PollState } from "./PollState";
import type { Polled } from "./Polled";
import { documentHidden } from "./documentHidden";


export function usePolled<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  intervalMs = 5000,
  key?: unknown,
): Polled<T> {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<ApiError | null>(null);
  const [state, setState] = useState<PollState>("connecting");
  const [updatedAt, setUpdatedAt] = useState<number | null>(null);
  const [paused, setPaused] = useState(false);

  const inFlight = useRef<AbortController | null>(null);
  const gen = useRef(0);
  const hidden = useRef(false);
  // §10: a 429 doubles the wait before the next tick (back to normal after
  // any success), so the console never hammers a throttled engine
  const backoffUntil = useRef(0);
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;
  const alive = useRef(true);

  const run = useCallback(async () => {
    if (inFlight.current) return; // overlap guard
    const ctrl = new AbortController();
    inFlight.current = ctrl;
    const myGen = gen.current;
    setState((s) => (s === "connecting" || s === "offline" ? s : "refreshing"));
    try {
      const d = await fetcherRef.current(ctrl.signal);
      if (!alive.current || myGen !== gen.current) return; // superseded query
      setData(d);
      setError(null);
      backoffUntil.current = 0;
      setUpdatedAt(Date.now());
      setState(documentHidden() ? "paused" : "live");
    } catch (e) {
      if (!alive.current) return;
      if (e instanceof ApiError && e.kind === "aborted") return; // superseded
      if ((e as { name?: string })?.name === "AbortError") return;
      const ae = e instanceof ApiError ? e : new ApiError("network", "admin API unreachable");
      setError(ae);
      if (ae.kind === "slowdown") {
        backoffUntil.current = Date.now() + 2 * Math.max(1500, intervalMs);
      }
      setState(ae.kind === "network" ? "offline" : "live");
    } finally {
      if (inFlight.current === ctrl) inFlight.current = null;
    }
  }, [intervalMs]);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      inFlight.current?.abort();
    };
  }, []);

  useEffect(() => {
    gen.current += 1; // bump generation so in-flight results of the old query die
    inFlight.current?.abort();
    inFlight.current = null;
    void run();
    const id = window.setInterval(() => {
      if (documentHidden() || paused) return;
      if (Date.now() < backoffUntil.current) return; // 429 cooldown
      void run();
    }, Math.max(1500, intervalMs));
    const onVis = () => {
      hidden.current = documentHidden();
      if (documentHidden()) {
        setState((s) => (s === "connecting" ? s : "paused"));
        inFlight.current?.abort();
        inFlight.current = null;
      } else {
        void run(); // catch up the moment we're visible again
      }
    };
    document.addEventListener("visibilitychange", onVis);
    return () => {
      window.clearInterval(id);
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [run, intervalMs, paused, key]);

  // honor external pause while idle
  useEffect(() => {
    setState((s) => {
      if (s === "connecting") return s;
      if (paused || documentHidden()) return "paused";
      if (inFlight.current) return "refreshing";
      // A failed poll must not overwrite "offline": unreachable is a state,
      // not a transient error. Other error kinds (http, slowdown) stay
      // "live" — the console is connected, the data is stale, and the
      // InlineError region tells that story separately.
      if (error) return error.kind === "network" ? "offline" : "live";
      return data !== null ? "live" : s;
    });
  }, [paused, data, error]);

  return { data, error, state, updatedAt, refresh: () => void run(), setPaused };
}
