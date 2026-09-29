// TransformList — split from TargetBuilder.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Button, Field, NumberInput, SelectInput, TagInput, TextInput, Toggle } from "../../../components/ui";
import { TRANSFORM_ARGS, TRANSFORM_OPS, type TransformOp } from "../../../lib/configSchema";
import { Icon } from "../../../components/Icons";
import type { TransformEntry } from "./TransformEntry";
import { emptyArgs } from "./emptyArgs";
import { argsReady } from "./argsReady";


export function TransformList({ ops, onChange }: { ops: TransformEntry[]; onChange: (l: TransformEntry[]) => void }) {
  // editor: { index: -1 = append, op, args }
  const [editor, setEditor] = useState<{ index: number; op: TransformOp; args: Record<string, unknown> } | null>(null);
  const move = (i: number, dir: -1 | 1) => {
    const n = [...ops];
    const j = i + dir;
    if (j < 0 || j >= n.length) return;
    [n[i], n[j]] = [n[j]!, n[i]!];
    onChange(n);
  };
  const start = (index: number, op: TransformOp) => {
    const existing = index >= 0 ? (ops[index] as TransformEntry) : null;
    setEditor({
      index,
      op,
      args: existing
        ? Object.fromEntries(Object.entries(existing).filter(([k]) => k !== "op"))
        : emptyArgs(op),
    });
  };
  const save = () => {
    if (!editor) return;
    const entry: TransformEntry = { op: editor.op, ...editor.args };
    onChange(editor.index < 0 ? [...ops, entry] : ops.map((o, i) => (i === editor.index ? entry : o)));
    setEditor(null);
  };
  const ready = editor ? argsReady(editor.op, editor.args) : true;

  return (
    <div>
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
        <span className="t-label">Transforms (applied in order)</span>
        <span style={{ marginLeft: "auto" }}>
          <button className="btn ghost sm" onClick={() => start(-1, "trim")} aria-expanded={!!editor && editor.index < 0}>
            + add op
          </button>
        </span>
      </div>
      {ops.length === 0 && !editor && <p className="t-caption">None — raw extraction values are kept.</p>}
      <ol style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 6 }}>
        {ops.map((op, i) => (
          <li
            key={i}
            style={{ display: "flex", gap: 8, alignItems: "center", border: "1px solid var(--line)", borderRadius: 7, padding: "6px 8px", background: "var(--bg-inset)" }}
          >
            <code className="t-code">{String(op.op)}</code>
            <span className="t-caption" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {Object.entries(op)
                .filter(([k]) => k !== "op")
                .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
                .join(" · ") || "no args"}
            </span>
            <span style={{ marginLeft: "auto", display: "flex", gap: 2 }}>
              <button className="iconbtn" aria-label={`move ${String(op.op)} up`} onClick={() => move(i, -1)}>
                <Icon.ChevronRight size={11} style={{ rotate: "-90deg" }} />
              </button>
              <button className="iconbtn" aria-label={`move ${String(op.op)} down`} onClick={() => move(i, 1)}>
                <Icon.ChevronRight size={11} style={{ rotate: "90deg" }} />
              </button>
              <button className="iconbtn" aria-label={`edit ${String(op.op)}`} onClick={() => start(i, (TRANSFORM_OPS as readonly string[]).includes(String(op.op)) ? (op.op as TransformOp) : "trim")}>
                <Icon.Config size={11} />
              </button>
              <button className="iconbtn" aria-label={`remove ${String(op.op)}`} onClick={() => onChange(ops.filter((_, x) => x !== i))}>
                <Icon.Close size={11} />
              </button>
            </span>
          </li>
        ))}
      </ol>
      {editor && (
        <div style={{ marginTop: 8, border: "1px dashed var(--line-strong)", borderRadius: 8, padding: 10, display: "grid", gap: 8 }}>
          <Field label="Operation" help="Exact TransformCfg enum from the engine config — 23 ops.">
            <SelectInput
              value={editor.op}
              onChange={(e) => {
                const op = e.target.value as TransformOp;
                setEditor({ ...editor, op, args: emptyArgs(op) });
              }}
            >
              {TRANSFORM_OPS.map((o) => (
                <option key={o} value={o}>{o}</option>
              ))}
            </SelectInput>
          </Field>
          {TRANSFORM_ARGS[editor.op].map((arg) => (
            <Field
              key={arg.name}
              label={`${arg.name}${arg.required ? "  (required)" : ""}`}
              error={arg.required && (editor.args[arg.name] ?? "") === "" ? "required" : null}
              help={arg.options ? `one of: ${arg.options.join(" | ")}` : arg.kind === "list" ? "Enter items one by one." : undefined}
            >
              {arg.kind === "number" ? (
                <NumberInput
                  value={typeof editor.args[arg.name] === "number" ? (editor.args[arg.name] as number) : 0}
                  onValue={(v) => setEditor({ ...editor, args: { ...editor.args, [arg.name]: v ?? 0 } })}
                />
              ) : arg.kind === "bool" ? (
                <Toggle checked={editor.args[arg.name] === true} onChange={(v) => setEditor({ ...editor, args: { ...editor.args, [arg.name]: v } })} label={arg.name} />
              ) : arg.kind === "list" ? (
                <TagInput
                  values={Array.isArray(editor.args[arg.name]) ? (editor.args[arg.name] as string[]) : []}
                  onChange={(v) => setEditor({ ...editor, args: { ...editor.args, [arg.name]: v } })}
                />
              ) : arg.options ? (
                <SelectInput value={String(editor.args[arg.name] ?? arg.options[0] ?? "")} onChange={(e) => setEditor({ ...editor, args: { ...editor.args, [arg.name]: e.target.value } })}>
                  {arg.options.map((o) => (
                    <option key={o} value={o}>{o}</option>
                  ))}
                </SelectInput>
              ) : (
                <TextInput
                  value={String(editor.args[arg.name] ?? "")}
                  placeholder={arg.default !== undefined ? String(arg.default) : undefined}
                  invalid={arg.required && editor.args[arg.name] === ""}
                  onChange={(e) => setEditor({ ...editor, args: { ...editor.args, [arg.name]: e.target.value } })}
                />
              )}
            </Field>
          ))}
          <div style={{ display: "flex", gap: 6, justifyContent: "flex-end" }}>
            <Button variant="ghost" onClick={() => setEditor(null)}>Cancel</Button>
            <Button variant="primary" disabled={!ready} onClick={save}>
              {editor.index < 0 ? `Add ${editor.op}` : `Save ${editor.op}`}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}
