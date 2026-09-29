// PipelineNode — split from patterns.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


/* ── Pipeline graph primitives ────────────────────────────────── */
export function PipelineNode({
  title,
  meta,
  sub,
  tone,
  selected,
  onSelect,
  as = "button",
}: {
  title: string;
  meta?: ReactNode;
  sub?: ReactNode;
  tone?: string;
  selected?: boolean;
  onSelect?: () => void;
  /** "div" renders the non-interactive variant (Target Builder strip) */
  as?: "button" | "div";
}) {
  const inner = (
    <>
      <span className="pn-title">
        {tone && <i className={`dot ${tone}`} aria-hidden="true" />}
        {title}
      </span>
      {meta && <span className="pn-meta">{meta}</span>}
      {sub && (
        <span className="t-caption pn-sub" style={{ display: "block", marginTop: 2 }}>
          {sub}
        </span>
      )}
    </>
  );
  if (as === "div") {
    return <span className="pipe-node" style={{ cursor: "default" }}>{inner}</span>;
  }
  return (
    <button type="button" className="pipe-node" aria-pressed={!!selected} onClick={onSelect}>
      {inner}
    </button>
  );
}
