// CommandPalette — split from Overlays.tsx; barrel re-exports keep every public path & name

import { useCallback, useRef, useState } from "react";
import { useTrap } from "./useTrap";
import type { PaletteItem } from "./PaletteItem";


export function CommandPalette({
  items,
  onClose,
}: {
  items: PaletteItem[];
  onClose: () => void;
}) {
  const [q, setQ] = useState("");
  const [active, setActive] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);

  const filtered = useCallback(() => {
    const needle = q.trim().toLowerCase();
    if (!needle) return items;
    return items.filter(
      (i) => i.label.toLowerCase().includes(needle) || i.hint?.toLowerCase().includes(needle),
    );
  }, [items, q])();


  const ref = useTrap(onClose);

  const groups = ["Navigate", "Actions", "Records"] as const;

  return (
    <>
      <div className="scrim" onClick={onClose} />
      <div className="palette" role="dialog" aria-modal="true" aria-label="Command palette" ref={ref}>
        <input
          autoFocus
          placeholder="Jump to a page, run an action, or search loaded records…"
          aria-label="Command search"
          value={q}
          onChange={(e) => { setQ(e.target.value); setActive(0); }}
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") { e.preventDefault(); setActive((a) => Math.min(filtered.length - 1, a + 1)); }
            if (e.key === "ArrowUp") { e.preventDefault(); setActive((a) => Math.max(0, a - 1)); }
            if (e.key === "Enter") {
              e.preventDefault();
              const it = filtered[active];
              if (it) { it.run(); onClose(); }
            }
          }}
        />
        <div className="palette-list" ref={listRef} role="listbox" aria-activedescendant={filtered[active]?.id}>
          {groups.map((g) => {
            const inG = filtered.filter((i) => i.group === g);
            if (!inG.length) return null;
            return (
              <div key={g}>
                <div className="t-label" style={{ padding: "8px 12px 4px" }}>{g}</div>
                {inG.map((i) => {
                  const idx = filtered.indexOf(i);
                  return (
                    <button
                      key={i.id}
                      id={i.id}
                      role="option"
                      aria-selected={idx === active}
                      data-active={idx === active ? "true" : undefined}
                      className="palette-item"
                      onMouseEnter={() => setActive(idx)}
                      onClick={() => { i.run(); onClose(); }}
                    >
                      {i.label}
                      {i.hint && <span className="hint">{i.hint}</span>}
                    </button>
                  );
                })}
              </div>
            );
          })}
          {filtered.length === 0 && (
            <div className="state" style={{ padding: 24 }}>
              <p>No matches in loaded data. (Search covers records the API already returned — it is not full-text.)</p>
            </div>
          )}
        </div>
      </div>
    </>
  );
}
