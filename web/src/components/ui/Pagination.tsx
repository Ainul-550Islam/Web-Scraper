// Pagination — split from ui.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";
import { Button } from "./Button";


/* ── pagination ───────────────────────────────────────────────── */

export function Pagination({
  total,
  limit,
  offset,
  onPage,
}: {
  total: number;
  limit: number;
  offset: number;
  onPage: (offset: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / limit));
  const cur = Math.floor(offset / limit) + 1;
  return (
    <nav className="pager" aria-label="pagination">
      <span className="tnum">
        {total === 0 ? "no entries" : `${Math.min(offset + 1, total)}–${Math.min(offset + limit, total)} of ${total}`}
      </span>
      <div className="pages">
        <Button className="sm" disabled={cur <= 1} onClick={() => onPage((cur - 2) * limit)} aria-label="previous page">
          <Icon.ChevronLeft size={12} />
        </Button>
        <span className="tnum" style={{ padding: "0 6px" }}>{cur} / {pages}</span>
        <Button className="sm" disabled={cur >= pages} onClick={() => onPage(cur * limit)} aria-label="next page">
          <Icon.ChevronRight size={12} />
        </Button>
      </div>
    </nav>
  );
}
