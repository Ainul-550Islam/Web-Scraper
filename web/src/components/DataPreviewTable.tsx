/**
 * DataPreviewTable — a bounded window onto a dataset, never the whole thing.
 *
 * Bounds are enforced here, not left to the caller: rows, columns and cell
 * characters all have caps, and the header says how much was withheld. Values
 * keep their JSON type (numbers are not stringified into "1", nulls are shown
 * as an em dash rather than the word "null"), and anything under a
 * credential-shaped key is masked with the house redaction mark before it can
 * reach the screen.
 */
import { useState } from "react";
import { deepMask, REDACTION_MARK } from "../lib/redaction";
import { fmtNum } from "../lib/dataFormatters";
import { CopyButton, EmptyState, Segmented } from "./ui";
import { JsonInspector } from "./patterns/JsonInspector";

export type PreviewRow = Record<string, unknown>;

export interface Props {
  columns: readonly string[];
  rows: readonly PreviewRow[];
  /** Rows rendered (default 25). */
  maxRows?: number;
  /** Columns rendered (default 12). */
  maxCols?: number;
  /** Characters kept per cell (default 48). */
  maxCellChars?: number;
  ariaLabel?: string;
}

type View = "table" | "json";

/**
 * JSON-typed rendering: no silent stringification of nulls or booleans.
 * Returns both the (possibly clipped) display text and the full value, so a
 * clipped cell can still expose everything on hover.
 */
// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function renderValue(v: unknown, maxChars: number): { text: string; full: string; kind: string; clipped: boolean } {
  if (v === null || v === undefined) return { text: "—", full: "", kind: "null", clipped: false };
  if (typeof v === "boolean") return { text: v ? "true" : "false", full: String(v), kind: "bool", clipped: false };
  if (typeof v === "number") return { text: String(v), full: String(v), kind: "number", clipped: false };
  const raw = typeof v === "string" ? v : (JSON.stringify(v) ?? "");
  const clipped = raw.length > maxChars;
  return {
    text: clipped ? `${raw.slice(0, maxChars)}…` : raw,
    full: raw,
    kind: typeof v === "string" ? "text" : Array.isArray(v) ? "list" : "object",
    clipped,
  };
}

export default function DataPreviewTable({
  columns,
  rows,
  maxRows = 25,
  maxCols = 12,
  maxCellChars = 48,
  ariaLabel = "data preview",
}: Props) {
  const [view, setView] = useState<View>("table");
  const shownCols = columns.slice(0, Math.max(1, maxCols));
  const shownRows = rows.slice(0, Math.max(1, maxRows));
  const hiddenCols = columns.length - shownCols.length;
  const hiddenRows = rows.length - shownRows.length;

  if (columns.length === 0 || rows.length === 0) {
    return <EmptyState title="No records to preview" body="Load a dataset (or clear the filters) to see a bounded preview here." />;
  }

  return (
    <div className="dpt">
      <div className="dpt-bar">
        <span className="t-caption t-muted">
          {fmtNum(shownRows.length)} of {fmtNum(rows.length)} row(s) · {fmtNum(shownCols.length)} of{" "}
          {fmtNum(columns.length)} column(s)
          {hiddenRows > 0 ? ` · ${fmtNum(hiddenRows)} row(s) not rendered` : ""}
          {hiddenCols > 0 ? ` · ${fmtNum(hiddenCols)} column(s) not rendered` : ""}
        </span>
        <Segmented
          ariaLabel="preview view"
          value={view}
          onChange={setView}
          options={[
            { value: "table", label: "Table" },
            { value: "json", label: "JSON" },
          ]}
        />
      </div>

      {view === "table" ? (
        <div className="dpt-scroll">
          <table className="data" aria-label={ariaLabel}>
            <thead>
              <tr>
                <th scope="col">#</th>
                {shownCols.map((c) => (
                  <th key={c} scope="col" title={c}>
                    {c}
                  </th>
                ))}
                <th scope="col">value</th>
              </tr>
            </thead>
            <tbody>
              {shownRows.map((row, i) => {
                const masked = deepMask(row) as PreviewRow;
                return (
                  <tr key={i}>
                    <td className="t-muted">{i + 1}</td>
                    {shownCols.map((c) => {
                      const raw = masked[c];
                      const { text, full, kind, clipped } = renderValue(raw, maxCellChars);
                      const maskedOut = raw === REDACTION_MARK;
                      return (
                        <td
                          key={c}
                          className={`dpt-cell ${kind}${maskedOut ? " redacted" : ""}${clipped ? " clipped" : ""}`}
                          title={maskedOut ? "redacted" : full}
                        >
                          {text}
                        </td>
                      );
                    })}
                    <td>
                      <CopyButton text={JSON.stringify(masked, null, 2)} label="Copy JSON" />
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="dpt-json">
          <JsonInspector value={deepMask(shownRows)} maxNodes={400} openDepth={1} redact />
          <CopyButton text={JSON.stringify(deepMask(shownRows), null, 2)} label="Copy preview JSON" />
        </div>
      )}
      <style>{`
        .dpt { display: flex; flex-direction: column; gap: 10px; }
        .dpt-bar { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; justify-content: space-between; }
        .dpt-scroll { max-width: 100%; overflow-x: auto; }
        .dpt-cell { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .dpt-cell.null { color: var(--ink-faint); }
        .dpt-cell.number { font-variant-numeric: tabular-nums; }
        .dpt-cell.bool { color: var(--ink-dim); }
        .dpt-cell.redacted { color: var(--warn-ink); letter-spacing: 0.08em; }
        .dpt-json { display: flex; flex-direction: column; gap: 8px; }
      `}</style>
    </div>
  );
}
