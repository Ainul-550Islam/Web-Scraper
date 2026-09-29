// PageHeader — split from ui.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


export function PageHeader({
  title,
  sub,
  actions,
}: {
  title: string;
  sub?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div style={{ display: "flex", gap: 16, alignItems: "flex-end", flexWrap: "wrap" }}>
      <div style={{ minWidth: 0 }}>
        <h1 className="t-page">{title}</h1>
        {sub && <p className="t-caption" style={{ marginTop: 2 }}>{sub}</p>}
      </div>
      {actions && <div style={{ marginLeft: "auto", display: "flex", gap: 8 }}>{actions}</div>}
    </div>
  );
}
