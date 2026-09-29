// formatVal — split from Diagnostics.tsx; barrel re-exports keep every public path & name

import { fmtMs, fmtNum } from "../../../lib/format";


export function formatVal(k: string, v: unknown): string {
  if (k.endsWith("_ms")) return fmtMs(Number(v));
  if (typeof v === "number") return fmtNum(v);
  if (typeof v === "boolean") return v ? "yes" : "no";
  return String(v);
}
