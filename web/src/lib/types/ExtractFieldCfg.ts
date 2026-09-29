// ExtractFieldCfg — split from types.ts; barrel re-exports keep every public path & name



/* ── config document (GET /config; secrets redacted server-side) ─ */
export interface ExtractFieldCfg {
  css: string;
  path: string | null;
  attr: string | null;
  all: boolean;
  transforms: unknown[];
}
