// Toasts — split from Overlays.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";
import { useToasts } from "../../hooks/useToasts";


/* ── toasts container ─────────────────────────────────────────── */
export function Toasts() {
  const { toasts, dismiss } = useToasts();
  return (
    <div className="toasts" aria-live="polite" role="region" aria-label="Notifications">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind === "info" ? "" : t.kind}`}>
          {t.kind === "ok" ? <Icon.Check size={14} /> : t.kind === "err" ? <Icon.Warn size={14} /> : <Icon.Info size={14} />}
          <span>{t.message}</span>
          <button className="iconbtn close" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
            <Icon.Close size={12} />
          </button>
        </div>
      ))}
    </div>
  );
}
