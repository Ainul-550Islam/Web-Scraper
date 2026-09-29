// AckRequest — split from types.ts; barrel re-exports keep every public path & name



/* ── POST bodies (the only two mutating routes) ──────────────── */
export interface AckRequest {
  id: string; // correlation id or "*" (bulk)
}
