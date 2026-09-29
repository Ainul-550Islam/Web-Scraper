// Field — split from ui.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";
import { Icon } from "../Icons";


/* ── form controls ────────────────────────────────────────────── */

export function Field({
  label,
  help,
  note,
  dirty,
  error,
  htmlFor,
  children,
  trailing,
}: {
  label: string;
  help?: string;
  note?: string;
  dirty?: boolean;
  error?: string | null;
  htmlFor?: string;
  children: ReactNode;
  trailing?: ReactNode;
}) {
  return (
    <div className="ctrl" data-dirty={dirty ? "true" : undefined}>
      <div className="ctrl-head">
        <label className="field-label" htmlFor={htmlFor}>
          {label}
          {note && <span className="field-note">{note}</span>}
        </label>
        {trailing}
      </div>
      {children}
      {help && <div className="field-help" id={htmlFor ? `${htmlFor}-help` : undefined}>{help}</div>}
      {error && (
        <div className="field-error" role="alert">
          <Icon.Warn size={12} /> {error}
        </div>
      )}
    </div>
  );
}
