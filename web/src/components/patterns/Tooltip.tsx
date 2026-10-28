// Tooltip — split from patterns.tsx; barrel re-exports keep every public path & name

import { useId, type ReactNode } from "react";


/* ── Tooltip (hover AND keyboard focus) ───────────────────────── */
export function Tooltip({ tip, children }: { tip: string; children: ReactNode }) {
  const id = `tt-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  return (
    <span className="tipwrap" tabIndex={0} aria-describedby={id}>
      {children}
      <span role="tooltip" id={id} className="tip">
        {tip}
      </span>
    </span>
  );
}
