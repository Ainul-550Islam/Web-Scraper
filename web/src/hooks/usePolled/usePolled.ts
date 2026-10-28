// usePolled — split from usePolled.ts; barrel re-exports keep every public path & name

import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError } from "../../lib/api";
import type { PollState } from "./PollState";
import type { Polled } from "./Polled";
import { documentHidden } from "./documentHidden";

type PollSnapshot<T> = {
  key: unknown;
  data: T | null;
  error: ApiError | null;
  state: PollState;
  updatedAt: number | null;
};

function emptySnapshot<T>(key: unknown, state: PollState = "connecting"): PollSnapshot<T> {
  return { key, data: null, error: null, state, updatedAt: null };
}

/** Poll one keyed resource without allowing stale requests to publish state.
 * A source/query change hides the old result during render, then aborts it;
 * external pause and hidden-tab state never schedule a fetch.
 */
export function usePolled<T>(
  fetcher: (signal: AbortSignal) => Promise<T>,
  intervalMs: number | ((data: T | null) => number) = 5000,
  key?: unknown,
): Polled<T> {
  const [snapshot, setSnapshot] = useState<PollSnapshot<T>>(() => emptySnapshot<T>(key));
  const [paused, setPausedState] = useState(false);
  const [hidden, setHidden] = useState(documentHidden);

  const inFlight = useRef<AbortController | null>(null);
  const generation = useRef(0);
  const pausedRef = useRef(false);
  const hiddenRef = useRef(false);
  const fetcherRef = useRef(fetcher);
  const alive = useRef(true);
  const backoffUntil = useRef(0);

  useEffect(() => { fetcherRef.current = fetcher; }, [fetcher]);
  useEffect(() => { pausedRef.current = paused; }, [paused]);
  useEffect(() => { hiddenRef.current = hidden; }, [hidden]);
  const visible = Object.is(snapshot.key, key) ? snapshot : emptySnapshot<T>(key);
  const effectiveInterval = Math.max(1500, typeof intervalMs === "function" ? intervalMs(visible.data) : intervalMs);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
      generation.current += 1;
      inFlight.current?.abort();
      inFlight.current = null;
    };
  }, []);

  const run = useCallback(async () => {
    if (pausedRef.current || hiddenRef.current || inFlight.current) return;
    const controller = new AbortController();
    inFlight.current = controller;
    const requestGeneration = generation.current;
    setSnapshot((current) => {
      const base = Object.is(current.key, key) ? current : emptySnapshot<T>(key);
      return { ...base, key, state: base.state === "connecting" || base.state === "offline" ? base.state : "refreshing" };
    });
    try {
      const data = await fetcherRef.current(controller.signal);
      if (!alive.current || requestGeneration !== generation.current || controller.signal.aborted) return;
      backoffUntil.current = 0;
      setSnapshot({ key, data, error: null, state: pausedRef.current || hiddenRef.current ? "paused" : "live", updatedAt: Date.now() });
    } catch (cause) {
      if (!alive.current || requestGeneration !== generation.current) return;
      if (cause instanceof ApiError && cause.kind === "aborted") return;
      if ((cause as { name?: string } | null)?.name === "AbortError") return;
      const error = cause instanceof ApiError ? cause : new ApiError("network", "admin API unreachable");
      if (error.kind === "slowdown") backoffUntil.current = Date.now() + 2 * effectiveInterval;
      setSnapshot((current) => ({
        ...emptySnapshot<T>(key, error.kind === "network" ? "offline" : "live"),
        data: Object.is(current.key, key) ? current.data : null,
        error,
        updatedAt: Object.is(current.key, key) ? current.updatedAt : null,
      }));
    } finally {
      if (inFlight.current === controller) inFlight.current = null;
    }
  }, [effectiveInterval, key]);

  useEffect(() => {
    generation.current += 1;
    inFlight.current?.abort();
    inFlight.current = null;
    if (!paused && !hidden) void run();
    const timer = window.setInterval(() => {
      if (paused || hidden || Date.now() < backoffUntil.current) return;
      void run();
    }, effectiveInterval);
    const onVisibility = () => setHidden(documentHidden());
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      generation.current += 1;
      inFlight.current?.abort();
      inFlight.current = null;
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [run, effectiveInterval, paused, hidden]);

  const refresh = useCallback(() => {
    generation.current += 1;
    inFlight.current?.abort();
    inFlight.current = null;
    if (!pausedRef.current && !hiddenRef.current) void run();
  }, [run]);

  const setPaused = useCallback((value: boolean) => {
    pausedRef.current = value;
    setPausedState(value);
  }, []);

  const state: PollState = paused || hidden ? "paused" : visible.state;
  return { data: visible.data, error: visible.error, state, updatedAt: visible.updatedAt, refresh, setPaused };
}
