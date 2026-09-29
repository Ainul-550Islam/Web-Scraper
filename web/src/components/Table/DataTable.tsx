// DataTable — split from Table.tsx; barrel re-exports keep every public path & name

import { useMemo, useRef, useState } from "react";
import { Icon } from "../Icons";
import { EmptyState, InlineError, Pagination, Skeleton } from "../ui";
import { Checkbox } from "../patterns";
import type { TableProps } from "./TableProps";
import { extract } from "./extract";


/**
 * One table for Runs/Jobs/Proxies/Audit/Alerts/Activity.
 * - sticky header, density toggle, column visibility, client sort
 * - roving tabindex keyboard navigation (↑/↓/Home/End, Enter opens)
 * - on narrow viewports the CSS collapses cells to labeled cards
 *   (data-col), so nothing is hidden behind horizontal scroll
 * - server paging when `total/onPage` given; otherwise client paging
 */
export function DataTable<Row>({
  caption,
  columns,
  rows,
  rowKey,
  total,
  limit = 25,
  offset = 0,
  onPage,
  loading = false,
  error = null,
  onRowClick,
  selectedKey = null,
  emptyTitle,
  emptyBody,
}: TableProps<Row>) {
  const [density, setDensity] = useState<"normal" | "compact">("normal");
  const [hidden, setHidden] = useState<Set<string>>(
    () => new Set(columns.filter((c) => c.defaultHidden).map((c) => c.key)),
  );
  const [colsMenu, setColsMenu] = useState(false);
  const [sort, setSort] = useState<{ key: string; dir: 1 | -1 } | null>(null);
  const [focusIdx, setFocusIdx] = useState(0);
  const tbodyRef = useRef<HTMLTableSectionElement>(null);

  const visible = columns.filter((c) => !hidden.has(c.key));

  const sorted = useMemo(() => {
    if (!sort) return rows;
    const col = columns.find((c) => c.key === sort.key);
    const sv = col?.sortValue ?? ((r: Row) => String(col ? extract(r, col) : ""));
    return [...rows].sort((a, b) => {
      const x = sv(a);
      const y = sv(b);
      return (x < y ? -1 : x > y ? 1 : 0) * sort.dir;
    });
  }, [rows, sort, columns]);

  const onKeyNav = (e: React.KeyboardEvent) => {
    if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    setFocusIdx((i) => {
      const n = sorted.length;
      if (!n) return 0;
      switch (e.key) {
        case "ArrowDown": return Math.min(n - 1, i + 1);
        case "ArrowUp": return Math.max(0, i - 1);
        case "Home": return 0;
        default: return n - 1;
      }
    });
    requestAnimationFrame(() =>
      tbodyRef.current?.querySelectorAll("tr")[focusIdx]?.querySelector<HTMLElement>("td,button")?.focus(),
    );
  };

  const noData = !loading && sorted.length === 0;

  return (
    <div className="card" onKeyDown={onKeyNav}>
      <div className="filterbar" style={{ justifyContent: "flex-end" }}>
        <span className="spacer" style={{ marginRight: "auto" }}>
          <span className="t-label">{caption}</span>
        </span>
        <button
          className="btn sm"
          aria-pressed={density === "compact"}
          onClick={() => setDensity(density === "compact" ? "normal" : "compact")}
        >
          Compact
        </button>
        <div style={{ position: "relative" }}>
          <button
            className="btn sm"
            aria-expanded={colsMenu}
            onClick={() => setColsMenu((v) => !v)}
          >
            Columns <Icon.ChevronRight size={11} style={{ rotate: "90deg" }} />
          </button>
          {colsMenu && (
            <div
              className="card"
              role="menu"
              aria-label="column visibility"
              style={{ position: "absolute", right: 0, top: "calc(100% + 6px)", zIndex: 30, width: 190, padding: 6 }}
              onMouseLeave={() => setColsMenu(false)}
            >
              {columns.filter((c) => c.hideable !== false).map((c) => (
                <Checkbox
                  key={c.key}
                  label={c.label}
                  checked={!hidden.has(c.key)}
                  disabled={!hidden.has(c.key) && visible.length <= 2}
                  onChange={() =>
                    setHidden((h) => {
                      const n = new Set(h);
                      if (n.has(c.key)) n.delete(c.key);
                      else if (visible.length > 2) n.add(c.key);
                      return n;
                    })
                  }
                />
              ))}
            </div>
          )}
        </div>
      </div>

      {error && <InlineError error={error} />}
      <div className={`table-wrap ${density === "compact" ? "compact" : ""}`}>
        {loading && sorted.length === 0 ? (
          <div style={{ padding: 16, display: "grid", gap: 10 }}>
            {[0, 1, 2, 3, 4].map((i) => <Skeleton key={i} h={18} />)}
          </div>
        ) : noData ? (
          <EmptyState icon={<Icon.Queue size={18} />} title={emptyTitle} body={emptyBody} />
        ) : (
          <table className="data" aria-rowcount={sorted.length}>
            <caption className="sr-only">{caption}</caption>
            <thead>
              <tr>
                {visible.map((c) => {
                  const active = sort?.key === c.key;
                  return (
                    <th key={c.key} scope="col" aria-sort={active ? (sort.dir === 1 ? "ascending" : "descending") : "none"}>
                      <button
                        className="sort"
                        onClick={() =>
                          setSort((s) =>
                            s?.key === c.key ? { key: c.key, dir: s.dir === 1 ? -1 : 1 } : { key: c.key, dir: -1 },
                          )
                        }
                      >
                        {c.label}
                        {active && (
                          <Icon.ChevronRight size={10} style={{ rotate: sort.dir === 1 ? "-90deg" : "90deg" }} />
                        )}
                      </button>
                    </th>
                  );
                })}
              </tr>
            </thead>
            <tbody ref={tbodyRef}>
              {sorted.map((r, idx) => {
                const key = rowKey(r);
                return (
                  <tr
                    key={key}
                    tabIndex={idx === focusIdx ? 0 : -1}
                    aria-selected={selectedKey === key}
                    onFocus={() => setFocusIdx(idx)}
                    className={onRowClick ? "clickable" : ""}
                    onClick={onRowClick ? () => onRowClick(r) : undefined}
                    onKeyDown={(e) => {
                      if (onRowClick && (e.key === "Enter" || e.key === " ")) {
                        e.preventDefault();
                        onRowClick(r);
                      }
                    }}
                  >
                    {visible.map((c) => (
                      <td key={c.key} data-col={c.label} className={c.numeric ? "num" : undefined}>
                        {c.render(r)}
                      </td>
                    ))}
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {onPage && (
        <Pagination total={total ?? sorted.length} limit={limit} offset={offset} onPage={onPage} />
      )}
    </div>
  );
}
