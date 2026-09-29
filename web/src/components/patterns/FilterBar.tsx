// FilterBar — split from patterns.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


/* ── FilterBar ────────────────────────────────────────────────── */
export function FilterBar({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`filterbar ${className}`}>{children}</div>;
}
