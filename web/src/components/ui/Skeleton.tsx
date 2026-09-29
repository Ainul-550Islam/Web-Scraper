// Skeleton — split from ui.tsx; barrel re-exports keep every public path & name



/* ── states ───────────────────────────────────────────────────── */

export function Skeleton({ h = 14, w = "100%" }: { h?: number; w?: string | number }) {
  return <div className="skeleton" style={{ height: h, width: w }} aria-hidden="true" />;
}
