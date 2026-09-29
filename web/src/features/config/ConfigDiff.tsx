/**
 * Config Diff — a READ-ONLY compare of two config documents.
 *
 * Nothing here writes to the server, applies a draft, or mutates either
 * input: the two panes are plain textareas, parsing happens locally, and the
 * diff is rendered from normalized JSON. This is deliberate — the engine has
 * no PATCH endpoint for configs (see the honest "not implemented" clauses in
 * GET /policy), so a diff tool that promised apply buttons would lie.
 *
 * Canonicalization: unknown keys sort deterministically (JSON.stringify with
 * sorted keys, like `harvest config inspect` prints), so reordering noise
 * never shows up as a change.
 */
import { useMemo, useState } from "react";
import { Card, CopyButton, PageHeader, TextArea } from "../../components/ui";
import { Page } from "../../components/AppShell";

type DiffOp = { kind: "same" | "add" | "del"; line: string };

/** Line-oriented pretty output for diffing: two spaces per level, every
 *  container on its own lines, scalars verbatim (never regex-mangled). */
function pretty(v: unknown, indent = 0): string {
  const pad = "  ".repeat(indent);
  const inner = "  ".repeat(indent + 1);
  if (Array.isArray(v)) {
    if (v.length === 0) return "[]";
    return `[\n${v.map((x) => `${inner}${pretty(x, indent + 1)}`).join(",\n")}\n${pad}]`;
  }
  if (v !== null && typeof v === "object") {
    const keys = Object.keys(v as Record<string, unknown>).sort();
    if (keys.length === 0) return "{}";
    const body = keys
      .map((k) => `${inner}${JSON.stringify(k)}: ${pretty((v as Record<string, unknown>)[k], indent + 1)}`)
      .join(",\n");
    return `{\n${body}\n${pad}}`;
  }
  return JSON.stringify(v) ?? "null";
}

function diffLines(a: string[], b: string[]): DiffOp[] {
  // classic LCS over ≤ LINES_CAP lines — configs are hundreds of lines, not
  // millions; anything larger is refused, not silently truncated
  const n = a.length;
  const m = b.length;
  const dp: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  const at = (i: number, j: number): number => dp[i]?.[j] ?? 0;
  for (let i = n - 1; i >= 0; i--)
    for (let j = m - 1; j >= 0; j--)
      dp[i]![j] = a[i] === b[j] ? at(i + 1, j + 1) + 1 : Math.max(at(i + 1, j), at(i, j + 1));
  const out: DiffOp[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      out.push({ kind: "same", line: a[i] ?? "" });
      i++;
      j++;
    } else if (at(i + 1, j) >= at(i, j + 1)) {
      out.push({ kind: "del", line: a[i++] ?? "" });
    } else {
      out.push({ kind: "add", line: b[j++] ?? "" });
    }
  }
  while (i < n) out.push({ kind: "del", line: a[i++] ?? "" });
  while (j < m) out.push({ kind: "add", line: b[j++] ?? "" });
  return out;
}

const LINES_CAP = 4000;

export default function ConfigDiff() {
  const [left, setLeft] = useState("");
  const [right, setRight] = useState("");

  const parsed = useMemo(() => {
    const parse = (src: string): { ok: true; value: unknown } | { ok: false; error: string } | { ok: true; value: undefined } => {
      if (src.trim() === "") return { ok: true, value: undefined };
      try {
        const v = JSON.parse(src);
        if (typeof v !== "object" || v === null) return { ok: false, error: "JSON root must be an object" };
        const lines = JSON.stringify(v).length;
        if (lines > 2_000_000) return { ok: false, error: "document exceeds the 2 MB compare budget" };
        return { ok: true, value: v };
      } catch (e) {
        return { ok: false, error: e instanceof Error ? e.message : String(e) };
      }
    };
    return { a: parse(left), b: parse(right) };
  }, [left, right]);

  const result = useMemo(() => {
    if (!parsed.a.ok || !parsed.b.ok) return null;
    if (parsed.a.value === undefined || parsed.b.value === undefined) return null;
    const la = pretty(parsed.a.value).split("\n");
    const lb = pretty(parsed.b.value).split("\n");
    if (la.length > LINES_CAP || lb.length > LINES_CAP) {
      return { tooBig: true as const, ops: [] as DiffOp[] };
    }
    return { tooBig: false as const, ops: diffLines(la, lb) };
  }, [parsed]);

  const diffText = useMemo(() => {
    if (!result || result.tooBig) return "";
    return result.ops
      .filter((o) => o.kind !== "same")
      .map((o) => `${o.kind === "add" ? "+" : "-"} ${o.line}`)
      .join("\n");
  }, [result]);

  const changed = result && !result.tooBig ? result.ops.filter((o) => o.kind !== "same").length : 0;

  return (
    <Page>
      <PageHeader
        title="Config Diff"
        sub="Paste two config documents (e.g. `harvest config inspect` output vs. your file) — comparison is local and read-only; nothing here can be applied."
      />
      <div className="grid grid-2">
        <Card title="Left (baseline)">
          <TextArea
            aria-label="left config document"
            value={left}
            onChange={(e) => setLeft(e.target.value)}
            rows={18}
            placeholder={'{ "name": "prod", ... }'}
            spellCheck={false}
          />
          {!parsed.a.ok && <p className="t-caption t-err" role="alert">left: {parsed.a.error}</p>}
        </Card>
        <Card title="Right (candidate)">
          <TextArea
            aria-label="right config document"
            value={right}
            onChange={(e) => setRight(e.target.value)}
            rows={18}
            placeholder={'{ "name": "prod", ... }'}
            spellCheck={false}
          />
          {!parsed.b.ok && <p className="t-caption t-err" role="alert">right: {parsed.b.error}</p>}
        </Card>
      </div>

      <Card
        title={`Difference${result && !result.tooBig ? ` — ${changed === 0 ? "identical after canonicalization" : `${changed} changed line(s)`}` : ""}`}
        actions={diffText ? <CopyButton text={diffText} label="Copy unified lines" /> : undefined}
      >
        {!result && (
          <p className="t-caption t-muted">
            Paste valid JSON in both panes to compute the diff. Keys are sorted first, so formatting
            and key order never count as changes.
          </p>
        )}
        {result?.tooBig && <p className="t-caption t-err" role="alert">documents exceed the {LINES_CAP}-line diff budget — compare smaller scopes (top-level keys) instead</p>}
        {result && !result.tooBig && (
          <pre className="cdiff" aria-label="line diff">
            {result.ops.map((o, i) => (
              <span key={i} className={`cdiff-${o.kind}`}>
                {o.kind === "add" ? "+ " : o.kind === "del" ? "- " : "  "}
                {o.line}
                {"\n"}
              </span>
            ))}
          </pre>
        )}
        <style>{`
          .cdiff { font-family: var(--mono, ui-monospace, monospace); font-size: 12px;
                   overflow: auto; max-height: 420px; margin: 0; white-space: pre-wrap; }
          .cdiff-add { display: block; color: var(--ok, inherit); }
          .cdiff-del { display: block; color: var(--err, inherit); }
          .cdiff-same { display: block; opacity: .65; }
        `}</style>
      </Card>
    </Page>
  );
}
