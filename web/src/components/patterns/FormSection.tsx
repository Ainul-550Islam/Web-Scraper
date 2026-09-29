// FormSection — split from patterns.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import { Icon } from "../Icons";


/* ── FormSection ──────────────────────────────────────────────── */
export function FormSection({
  id,
  title,
  count,
  badge,
  warning,
  defaultOpen = true,
  children,
}: {
  id: string;
  title: string;
  count?: number;
  badge?: ReactNode;
  warning?: ReactNode;
  defaultOpen?: boolean;
  children: ReactNode;
}) {
  return (
    <details className="sec" id={`sec-${id}`} open={defaultOpen}>
      <summary>
        <Icon.ChevronRight size={12} className="chev" />
        {title}
        {count !== undefined && <span className="t-caption">{count} control(s)</span>}
        {badge}
      </summary>
      <div className="sec-body">
        {warning && (
          <p className="sec-warning" role="note">
            <Icon.Warn size={12} /> {warning}
          </p>
        )}
        {children}
      </div>
    </details>
  );
}
