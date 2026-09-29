// MiniBars — split from ui.tsx; barrel re-exports keep every public path & name



export function MiniBars({
  values,
  label,
}: {
  values: number[];
  label?: string;
}) {
  const max = Math.max(...values, 1);
  return (
    <div className="bars" role="img" aria-label={label ?? "recent samples"}>
      {values.map((v, i) => (
        <i key={i} className={v === 0 ? "empty" : ""} style={{ height: `${Math.max(8, (v / max) * 100)}%` }} />
      ))}
    </div>
  );
}
