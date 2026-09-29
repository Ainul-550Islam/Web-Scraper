// AlertItem — split from types.ts; barrel re-exports keep every public path & name



/* ── alerts / ops / audit ─────────────────────────────────────── */
export interface AlertItem {
  id?: string;
  key?: string;
  kind?: string;
  severity?: string;
  ts?: number;
  message?: string;
  [k: string]: unknown;
}
