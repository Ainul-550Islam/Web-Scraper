// MetricCard — split from ui.tsx; barrel re-exports keep every public path & name

import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Tone } from "./Tone";
import { Spark } from "./Spark";


/* ── metric card ──────────────────────────────────────────────── */

export function MetricCard({
  label,
  value,
  unit,
  meta,
  tone,
  spark,
  live = false,
}: {
  label: string;
  value: ReactNode;
  unit?: string;
  meta?: ReactNode;
  tone?: Tone;
  spark?: number[];
  live?: boolean;
}) {
  const shown = String(value);
  const [bump, setBump] = useState(false);
  const prev = useRef(shown);
  useEffect(() => {
    if (prev.current !== shown) {
      prev.current = shown;
      setBump(true);
      const h = setTimeout(() => setBump(false), 240);
      return () => clearTimeout(h);
    }
  }, [shown]);
  return (
    <article className="card metric card--hover">
      <div className="metric-label t-label" style={{ display: "flex", alignItems: "center", gap: 6 }}>
        {tone && <i className={`dot ${tone}`} aria-hidden="true" />}
        {label}
        {live && <i className="dot ok live" style={{ marginLeft: "auto" }} aria-hidden="true" />}
      </div>
      <div className={`metric-value t-metric t-metric-xl tnum ${bump ? "bump" : ""}`} aria-live="off">
        {value}
        {unit && <small>{unit}</small>}
      </div>
      <div className="metric-meta t-caption">
        {meta}
        {spark && spark.length > 1 && (
          <span style={{ marginLeft: "auto", width: 72 }}>
            <Spark values={spark} height={20} />
          </span>
        )}
      </div>
    </article>
  );
}
