// ComponentStatus — split from types.ts; barrel re-exports keep every public path & name



/* ── component health (workers/queue endpoints) ─────────────── */
export type ComponentStatus =
  | "healthy"
  | "degraded"
  | "unavailable"
  | "disabled";
