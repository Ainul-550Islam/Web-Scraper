// Checkbox — split from patterns.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";


/* ── Checkbox ─────────────────────────────────────────────────── */
export function Checkbox({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <label className="checkbox">
      <input type="checkbox" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} />
      <span aria-hidden="true" className="box">
        {checked && <Icon.Check size={10} />}
      </span>
      {label}
    </label>
  );
}
