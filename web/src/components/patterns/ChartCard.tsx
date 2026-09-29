// ChartCard — split from patterns.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


/* ── ChartCard ────────────────────────────────────────────────── */
export function ChartCard({
  title,
  meta,
  children,
  actions,
}: {
  title: string;
  meta?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <section className="card chart-card">
      <header className="card-head">
        <h2 className="t-section">{title}</h2>
        {meta && <span className="t-caption">{meta}</span>}
        <span className="spacer" />
        {actions}
      </header>
      <div className="card-body chart-body">{children}</div>
    </section>
  );
}
