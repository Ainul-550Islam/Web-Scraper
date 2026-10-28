// SearchInput — split from patterns.tsx; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";
import { Icon } from "../Icons";
import { useDebounced } from "../../hooks/useDebounced";


/* ── SearchInput ──────────────────────────────────────────────── */
export function SearchInput({
  value,
  onChange,
  placeholder,
  label,
  debounceMs = 200,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  label: string;
  debounceMs?: number;
}) {
  const [draft, setDraft] = useState<{ source: string; value: string }>({ source: value, value });
  const local = draft.source === value ? draft.value : value;
  const deb = useDebounced(local, debounceMs);
  useEffect(() => {
    if (deb === local && deb !== value) onChange(deb);
  }, [deb, local, value, onChange]);
  return (
    <span className="search">
      <Icon.Search size={12} />
      <input
        className="input"
        style={{ height: 30, paddingLeft: 30 }}
        type="search"
        aria-label={label}
        placeholder={placeholder}
        value={local}
        onChange={(e) => setDraft({ source: value, value: e.target.value })}
      />
    </span>
  );
}
