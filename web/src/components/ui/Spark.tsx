// Spark — split from ui.tsx; barrel re-exports keep every public path & name



/* ── viz ──────────────────────────────────────────────────────── */

export function Spark({
  values,
  height = 42,
  label,
}: {
  values: number[];
  height?: number;
  label?: string;
}) {
  const w = 120;
  if (values.length < 2) return null;
  const min = Math.min(...values);
  const max = Math.max(...values);
  const span = max - min || 1;
  const pts = values.map(
    (v, i) =>
      [
        (i / (values.length - 1)) * w,
        height - 3 - ((v - min) / span) * (height - 6),
      ] as const,
  );
  const d = pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join(" ");
  const last = pts[pts.length - 1] as readonly [number, number];
  return (
    <svg className="spark" viewBox={`0 0 ${w} ${height}`} role="img"
      aria-label={label ?? `trend, ${values.length} samples`} preserveAspectRatio="none">
      <defs>
        <linearGradient id="sparkfill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="var(--accent)" stopOpacity="0.22" />
          <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <path className="area" d={`${d} L ${w} ${height} L 0 ${height} Z`} />
      <path className="line" d={d} />
      <circle className="last" cx={last[0]} cy={last[1]} r="2.2" />
    </svg>
  );
}
