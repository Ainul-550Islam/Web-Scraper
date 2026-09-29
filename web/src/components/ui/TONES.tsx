// TONES — split from ui.tsx; barrel re-exports keep every public path & name

import type { Tone } from "./Tone";

export const TONES: Record<string, Tone> = {
  ok: "ok", running: "info", healthy: "ok", completed: "ok", done: "ok",
  degraded: "warn", retrying: "warn", warning: "warn", pending: "warn",
  error: "err", failed: "err", unavailable: "err", critical: "err",
  disabled: "idle", idle: "idle", paused: "idle", cancelled: "idle",
  drained: "idle", queued: "info",
};
