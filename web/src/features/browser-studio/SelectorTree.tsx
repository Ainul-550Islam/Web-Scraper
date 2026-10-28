import { Button, EmptyState } from "../../../components/ui";
import type { SelectorField } from "./SelectorEditor";

function renderNode(node: SelectorField, depth: number, selected: string | null, onSelect: (id: string) => void) {
  return <li key={node.id} style={{ marginInlineStart: Math.min(depth * 16, 128) }}>
    <button type="button" className="tree-node" aria-current={selected === node.id ? "true" : undefined} onClick={() => onSelect(node.id)}>
      <strong>{node.name || "unnamed field"}</strong><span className="t-muted">{node.kind}{node.repeat ? " · repeat" : ""}</span><code>{node.selector || "(selector required)"}</code>
    </button>
    {node.children.length > 0 && <ul>{node.children.map((child) => renderNode(child, depth + 1, selected, onSelect))}</ul>}
  </li>;
}

export default function SelectorTree({ nodes, selectedId, onSelect, onAddField, onAddRecord, onAddChild, onMove, onDuplicate, onDelete }: {
  nodes: SelectorField[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  onAddField: () => void;
  onAddRecord: () => void;
  onAddChild: () => void;
  onMove: (id: string, direction: -1 | 1) => void;
  onDuplicate: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  const selected = findNode(nodes, selectedId);
  return <div>
    <div className="row-actions">
      <Button onClick={onAddField}>Add field</Button><Button onClick={onAddRecord}>Add repeated record</Button><Button disabled={!selected} onClick={onAddChild}>Add child</Button>
      {selected && <><Button onClick={() => onMove(selected.id, -1)}>Move up</Button><Button onClick={() => onMove(selected.id, 1)}>Move down</Button><Button onClick={() => onDuplicate(selected.id)}>Duplicate</Button><Button variant="danger" onClick={() => onDelete(selected.id)}>Delete</Button></>}
    </div>
    {nodes.length === 0 ? <EmptyState title="Selector tree is empty" body="Pick a page element, add a field, or create a repeated record container." /> : <ul className="selector-tree">{nodes.map((node) => renderNode(node, 0, selectedId, onSelect))}</ul>}
  </div>;
}

export function findNode(nodes: SelectorField[], id: string | null): SelectorField | null {
  if (!id) return null;
  for (const node of nodes) { if (node.id === id) return node; const child = findNode(node.children, id); if (child) return child; }
  return null;
}
