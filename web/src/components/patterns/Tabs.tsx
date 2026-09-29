// Tabs — split from patterns.tsx; barrel re-exports keep every public path & name

import { useRef } from "react";


/* ── Tabs ─────────────────────────────────────────────────────── */
export function Tabs<T extends string>({
  value,
  onChange,
  options,
  ariaLabel,
}: {
  value: T;
  onChange: (v: T) => void;
  options: readonly { value: T; label: string }[];
  ariaLabel: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const onKey = (e: React.KeyboardEvent) => {
    const i = options.findIndex((o) => o.value === value);
    if (e.key === "ArrowRight" || e.key === "ArrowLeft") {
      e.preventDefault();
      const n = (i + (e.key === "ArrowRight" ? 1 : options.length - 1)) % options.length;
      onChange(options[n]!.value);
      ref.current?.querySelectorAll<HTMLButtonElement>("button")[n]?.focus();
    }
  };
  return (
    <div ref={ref} className="tabs" role="tablist" aria-label={ariaLabel} onKeyDown={onKey}>
      {options.map((o) => (
        <button
          key={o.value}
          role="tab"
          type="button"
          className={`tab ${o.value === value ? "on" : ""}`}
          aria-selected={o.value === value}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}
