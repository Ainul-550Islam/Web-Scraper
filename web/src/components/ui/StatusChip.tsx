// StatusChip — split from ui.tsx; barrel re-exports keep every public path & name

import { toneFor } from "./toneFor";


export function StatusChip({
  status,
  dot = true,
  label,
}: {
  status: string;
  dot?: boolean;
  label?: string;
}) {
  return (
    <span className={`chip ${toneFor(status)}`}>
      {dot && <i className={`dot ${toneFor(status)}`} aria-hidden="true" />}
      {label ?? status}
    </span>
  );
}
