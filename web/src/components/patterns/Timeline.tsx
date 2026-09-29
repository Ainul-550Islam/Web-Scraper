// Timeline — split from patterns.tsx; barrel re-exports keep every public path & name

import type { TimelineItem } from "./TimelineItem";


export function Timeline({ items, label }: { items: TimelineItem[]; label?: string }) {
  return (
    <ol className="timeline" role="list" aria-label={label ?? "timeline"}>
      {items.map((it, i) => (
        <li key={i} className={`tl-row ${it.tone ?? ""}`}>
          <span className="tl-dot" aria-hidden="true" />
          <div className="tl-title">{it.title}</div>
          {it.meta ? <div className="tl-meta">{it.meta}</div> : null}
        </li>
      ))}
    </ol>
  );
}
