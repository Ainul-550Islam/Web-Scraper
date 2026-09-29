// RunInfo — split from types.ts; barrel re-exports keep every public path & name



/* ── runs (paged history) ─────────────────────────────────────── */
export interface RunInfo {
  config_sha256: string;
  duration_ms: number;
  error: string | null;
  index: number;
  quarantined: number;
  run: string;
  status: string;
  ts: number;
  written: number;
}
