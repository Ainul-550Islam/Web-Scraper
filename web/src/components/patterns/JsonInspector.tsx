/**
 * JsonInspector — bounded, redaction-first JSON viewer for event details,
 * API payloads, and pasted documents.
 *
 * Two promises, both tested:
 *  1. Everything DISPLAYED passes `deepMask` first (nested credential keys
 *     and userinfo URLs never reach the DOM);
 *  2. Everything COPIED is the redacted serialization — not the raw value —
 *     so the clipboard can't leak what the screen refused to show.
 *
 * Rendering is strictly text nodes inside <pre>/<span>; no
 * dangerouslySetInnerHTML anywhere, so hostile JSON (HTML in strings, long
 * keys) is inert. `maxNodes` bounds cost for pathological payloads; beyond
 * the budget, deeper values collapse to a counted stub.
 */
import { useMemo, useState } from "react";
import { deepMask } from "../../lib/redaction";
import { Card, CopyButton } from "../ui";

interface Props {
  value: unknown;
  /** total rendered scalar/object nodes before collapsing the rest (default 500) */
  maxNodes?: number;
  /** default-open depth for containers (default 2) */
  openDepth?: number;
  /** when false, show raw (only safe for already-trusted local content) */
  redact?: boolean;
}

const INDENT = 14; // px per level — inline style keeps it theme-independent

function Leaf({ k, v }: { k: string | null; v: unknown }) {
  const text =
    typeof v === "string" ? JSON.stringify(v) : v === undefined ? "undefined" : String(v);
  return (
    <div className="ji-row">
      {k !== null && <span className="ji-key">{k}: </span>}
      <span className={typeof v === "string" ? "ji-str" : "ji-lit"}>{text}</span>
    </div>
  );
}

function Node({
  name,
  value,
  depth,
  openDepth,
  budget,
}: {
  name: string | null;
  value: unknown;
  depth: number;
  openDepth: number;
  budget: { left: number; collapsed: number };
}) {
  const [open, setOpen] = useState(depth < openDepth);
  const style = { paddingLeft: depth * INDENT + 8 };
  if (budget.left <= 0) {
    budget.collapsed += 1;
    return (
      <div className="ji-row ji-muted" style={style}>
        {name !== null && <span className="ji-key">{name}: </span>}
        <span>[collapsed — node budget exceeded]</span>
      </div>
    );
  }
  budget.left -= 1;
  if (value === null || typeof value !== "object") return <div style={style}><Leaf k={name} v={value} /></div>;
  if (Array.isArray(value)) {
    budget.left -= value.length;
    return (
      <div style={style}>
        <button type="button" className="ji-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
          {open ? "▾" : "▸"} {name !== null ? `${name}: ` : ""}[{value.length}]
        </button>
        {open &&
          value.map((v, i) => (
            <Node key={i} name={String(i)} value={v} depth={depth + 1} openDepth={openDepth} budget={budget} />
          ))}
      </div>
    );
  }
  const entries = Object.entries(value as Record<string, unknown>);
  budget.left -= entries.length;
  return (
    <div style={style}>
      <button type="button" className="ji-toggle" aria-expanded={open} onClick={() => setOpen(!open)}>
        {open ? "▾" : "▸"} {name !== null ? `${name}: ` : ""}{"{"}{entries.length}{"}"}
      </button>
      {open &&
        entries.map(([k, v]) => (
          <Node key={k} name={k} value={v} depth={depth + 1} openDepth={openDepth} budget={budget} />
        ))}
    </div>
  );
}

export function JsonInspector({ value, maxNodes = 500, openDepth = 2, redact = true }: Props) {
  // redact BEFORE display — the tree below only ever receives masked copies
  const masked = useMemo(() => (redact ? deepMask(value) : value), [value, redact]);
  const copyText = useMemo(() => JSON.stringify(masked, null, 2), [masked]);
  // fresh budget per render: the node walk mutates it as it goes
  const budget = { left: maxNodes, collapsed: 0 };
  return (
    <Card
      title="JSON"
      actions={
        <CopyButton
          text={copyText}
          label="Copy redacted JSON"
        />
      }
    >
      <div className="ji-root">
        <Node name={null} value={masked} depth={0} openDepth={openDepth} budget={budget} />
        {budget.collapsed > 0 && (
          <p className="t-caption ji-muted">
            {budget.collapsed} node(s) beyond the {maxNodes}-node render budget.
          </p>
        )}
      </div>
      <style>{`
        .ji-root { font-family: var(--mono, ui-monospace, monospace); font-size: 12.5px;
                   overflow-x: auto; }
        .ji-row  { white-space: pre-wrap; word-break: break-word; }
        .ji-key  { opacity: .75; }
        .ji-str  { color: var(--ok, inherit); }
        .ji-lit  { color: var(--info, inherit); }
        .ji-toggle { background: none; border: 0; color: inherit; cursor: pointer;
                     font: inherit; padding: 0; text-align: left; }
        .ji-muted  { opacity: .6; }
      `}</style>
    </Card>
  );
}
