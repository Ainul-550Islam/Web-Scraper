// TimelineItem — split from patterns.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


/* ── Timeline ─────────────────────────────────────────────────── */
export interface TimelineItem {
  title: string;
  meta?: ReactNode;
  tone?: "ok" | "info" | "warn" | "err" | "";
}
