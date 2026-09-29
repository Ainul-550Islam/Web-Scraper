// StudioIndex — computed navigation over the control registry: one row per
// section with its real control count, dirty count, and validation-error
// count. Every number is derived from CONTROLS_BY_SECTION + the live draft,
// so the "37 supported configuration controls" figure can never be stale.

import { CONTROLS, CONTROLS_BY_SECTION } from "../../../lib/configSchema";

export function StudioIndex({
  dirtyPaths,
  fieldErrors,
}: {
  dirtyPaths: Set<string>;
  fieldErrors: Map<string, string>;
}) {
  const rows = CONTROLS_BY_SECTION.map(({ section, controls }) => ({
    id: section.toLowerCase(),
    label: section,
    n: controls.length,
    dirty: controls.filter((c) => dirtyPaths.has(c.path)).length,
    bad: controls.filter((c) => fieldErrors.has(c.path)).length,
  }));
  return (
    <nav className="card" aria-label="Studio index" style={{ padding: "10px 14px" }}>
      <div style={{ display: "flex", flexWrap: "wrap", gap: "4px 14px", alignItems: "baseline" }}>
        <span className="t-label" style={{ marginRight: 6 }}>
          {CONTROLS.length} supported configuration controls
        </span>
        {rows.map((r) => (
          <a
            key={r.id}
            className="t-caption"
            href={`#sec-${r.id}`}
            style={{ display: "inline-flex", gap: 6, alignItems: "baseline", color: "inherit" }}
          >
            <strong>{r.label}</strong>
            <span className="tnum">{r.n}</span>
            {r.dirty > 0 && <span className="chip warn">{r.dirty} edited</span>}
            {r.bad > 0 && <span className="chip err" role="status">{r.bad} invalid</span>}
          </a>
        ))}
      </div>
    </nav>
  );
}
