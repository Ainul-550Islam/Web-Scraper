// SearchInput — split from patterns.tsx; barrel re-exports keep every public path & name

import { useEffect, useRef, useState } from "react";
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
  const [local, setLocal] = useState(value);
  const deb = useDebounced(local, debounceMs);
  const skip = useRef(true);
  useEffect(() => setLocal(value), [value]);
  useEffect(() => {
    if (skip.current) {
      skip.current = false;
      return;
    }
    onChange(deb);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [deb]);
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
        onChange={(e) => setLocal(e.target.value)}
      />
    </span>
  );
}
