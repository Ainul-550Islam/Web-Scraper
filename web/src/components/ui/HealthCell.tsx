// HealthCell — split from ui.tsx; barrel re-exports keep every public path & name

import { toneFor } from "./toneFor";


export function HealthCell({
  name,
  status,
  detail,
}: {
  name: string;
  status: string;
  detail?: string;
}) {
  const cls =
    status === "healthy" ? "ok" : status === "disabled" ? "disabled" : status === "degraded" ? "degraded" : "err";
  return (
    <div className={`health-cell ${cls}`}>
      <i className={`dot ${toneFor(status)}`} aria-hidden="true" />
      <div style={{ minWidth: 0 }}>
        <div className="hc-name">{name}</div>
        <div className="hc-state">{detail ?? status}</div>
      </div>
    </div>
  );
}
