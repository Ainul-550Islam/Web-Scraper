// Tooltip — split from patterns.tsx; barrel re-exports keep every public path & name

import { useRef, type ReactNode } from "react";


/* ── Tooltip (hover AND keyboard focus) ───────────────────────── */
export function Tooltip({ tip, children }: { tip: string; children: ReactNode }) {
  const id = useRef(`tt-${Math.random().toString(36).slice(2, 8)}`).current;
  return (
    <span className="tipwrap" tabIndex={0} aria-describedby={id}>
      {children}
      <span role="tooltip" id={id} className="tip">
        {tip}
      </span>
    </span>
  );
}
