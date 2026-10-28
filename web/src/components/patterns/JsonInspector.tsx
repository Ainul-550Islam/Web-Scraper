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
import { deepMask, MAX_MASK_NODES } from "../../lib/redaction";
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

type TreeNode = {
  name: string | null;
  value?: unknown;
  children?: TreeNode[];
  length?: number;
  array?: boolean;
  collapsed?: boolean;
};

function buildTree(value: unknown, maxNodes: number, maxDepth: number): { root: TreeNode; omitted: number } {
  let visited = 0;
  let omitted = 0;
  function visit(name: string | null, current: unknown, depth: number): TreeNode {
    if (visited >= maxNodes) {
      omitted += 1;
      return { name, collapsed: true };
    }
    // Count depth-limit stubs too. Otherwise a very wide object just above
    // the depth boundary can allocate/render an unbounded number of stubs.
    visited += 1;
    if (depth >= maxDepth) {
      omitted += 1;
      return { name, collapsed: true };
    }
    if (current === null || typeof current !== "object") return { name, value: current };
    if (Array.isArray(current)) {
      const children: TreeNode[] = [];
      for (let i = 0; i < current.length && visited < maxNodes; i += 1) children.push(visit(String(i), current[i], depth + 1));
      if (children.length < current.length) {
        omitted += 1;
        children.push({ name: null, collapsed: true });
      }
      return { name, length: current.length, array: true, children };
    }
    const children: TreeNode[] = [];
    for (const key in current) {
      if (!Object.hasOwn(current, key)) continue;
      if (visited >= maxNodes) { omitted += 1; children.push({ name: null, collapsed: true }); break; }
      children.push(visit(key, (current as Record<string, unknown>)[key], depth + 1));
    }
    return { name, length: children.length, children };
  }
  const root = visit(null, value, 0);
  return { root, omitted };
}

function Node({ node, depth, openDepth }: { node: TreeNode; depth: number; openDepth: number }) {
  const [open, setOpen] = useState(depth < openDepth);
  const style = { paddingLeft: depth * INDENT + 8 };
  if (node.collapsed) {
    return <div className="ji-row ji-muted" style={style}><span>[collapsed — node/depth budget exceeded]</span></div>;
  }
  if (!node.children) return <div style={style}><Leaf k={node.name} v={node.value} /></div>;
  const array = node.array === true;
  return (
    <div style={style}>
      <button type="button" className="ji-toggle" aria-expanded={open} onClick={() => setOpen((value) => !value)}>
        {open ? "▾" : "▸"} {node.name !== null ? `${node.name}: ` : ""}{array ? `[${node.length}]` : `{${node.length}}`}
      </button>
      {open && node.children.map((child, index) => <Node key={child.name ?? `omitted-${index}`} node={child} depth={depth + 1} openDepth={openDepth} />)}
    </div>
  );
}

export function JsonInspector({ value, maxNodes = 500, openDepth = 2, redact = true }: Props) {
  const nodeBudget = Number.isFinite(maxNodes)
    ? Math.max(1, Math.min(MAX_MASK_NODES, Math.floor(maxNodes)))
    : 500;
  const safeOpenDepth = Number.isFinite(openDepth)
    ? Math.max(0, Math.min(64, Math.floor(openDepth)))
    : 2;
  // Redact and bound BEFORE display/copy. Neither traversal may walk an
  // attacker-sized tree just to produce a later bounded rendering.
  const masked = useMemo(() => (redact ? deepMask(value, nodeBudget) : value), [value, redact, nodeBudget]);
  const copyText = useMemo(() => JSON.stringify(masked, null, 2), [masked]);
  // Precompute a bounded immutable tree; components only render that value.
  const tree = useMemo(() => buildTree(masked, nodeBudget, 64), [masked, nodeBudget]);
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
        <Node node={tree.root} depth={0} openDepth={safeOpenDepth} />
        {tree.omitted > 0 && (
          <p className="t-caption ji-muted">
            {tree.omitted} node(s) beyond the {nodeBudget}-node render budget.
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
