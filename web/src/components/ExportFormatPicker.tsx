/**
 * ExportFormatPicker — the one place the console states what a delivery can
 * be. Cards are factual: name, extension, what the format is *for*, whether
 * this engine build writes it, and what the current selection would produce.
 *
 * The registry mirrors `harvest::export::ExportFormat` (src/export.rs) and is
 * pinned from both sides: `tests/export_formats.rs::format_registry_is_honest`
 * (Rust) and `web/src/test/export-center.test.tsx` (this picker). A format the
 * engine does not implement is rendered as unavailable — never as a card that
 * silently produces nothing.
 */
import { StatusChip } from "./ui";

export type ExportFormatId = "csv" | "jsonl" | "xlsx" | "pdf" | "parquet" | "xml" | "zip";

export interface ExportFormatDef {
  id: ExportFormatId;
  name: string;
  ext: string;
  /** Factual one-liner: what a client does with this file. */
  purpose: string;
  /** Shape of the output, so the estimate line can be written honestly. */
  kind: "text-table" | "record-stream" | "workbook" | "report" | "columnar" | "interchange" | "archive";
}

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export const EXPORT_FORMATS: readonly ExportFormatDef[] = [
  { id: "csv", name: "CSV", ext: ".csv", purpose: "Wide spreadsheet-shaped text, one row per record", kind: "text-table" },
  { id: "jsonl", name: "JSONL", ext: ".jsonl", purpose: "Canonical record stream — the manifest's source of truth", kind: "record-stream" },
  { id: "xlsx", name: "XLSX", ext: ".xlsx", purpose: "Typed Excel workbook for business review", kind: "workbook" },
  { id: "pdf", name: "PDF", ext: ".pdf", purpose: "Summary report: identity, quality, integrity, sample", kind: "report" },
  { id: "parquet", name: "Parquet", ext: ".parquet", purpose: "Columnar analytics file (pandas, DuckDB, Spark)", kind: "columnar" },
  { id: "xml", name: "XML", ext: ".xml", purpose: "System interchange with escaped, deterministic elements", kind: "interchange" },
  { id: "zip", name: "ZIP Bundle", ext: ".zip", purpose: "One download: selected formats + manifest + checksums", kind: "archive" },
];

export interface Props {
  /** Format ids the connected engine can actually write. */
  available: readonly ExportFormatId[];
  selected: readonly ExportFormatId[];
  onToggle: (id: ExportFormatId) => void;
  /** Factual estimate per format (rows/sheets/files) — never a guess at bytes. */
  estimate?: (id: ExportFormatId) => string | null;
  disabled?: boolean;
  /** True while the source dataset is still being read. */
  loading?: boolean;
}

export default function ExportFormatPicker({
  available,
  selected,
  onToggle,
  estimate,
  disabled = false,
  loading = false,
}: Props) {
  return (
    <div className="fmt-grid" role="group" aria-label="export formats">
      {EXPORT_FORMATS.map((f) => {
        const supported = available.includes(f.id);
        const isSelected = selected.includes(f.id);
        const est = estimate?.(f.id) ?? null;
        return (
          <button
            key={f.id}
            type="button"
            className={`fmt-card${isSelected ? " on" : ""}${supported ? "" : " off"}`}
            aria-pressed={isSelected}
            disabled={disabled || loading || !supported}
            data-format={f.id}
            data-supported={supported ? "yes" : "no"}
            onClick={() => onToggle(f.id)}
          >
            <span className="fmt-head">
              <span className="fmt-name">{f.name}</span>
              <code className="fmt-ext">{f.ext}</code>
            </span>
            <span className="fmt-purpose">{f.purpose}</span>
            <span className="fmt-foot">
              {supported ? (
                <StatusChip status={isSelected ? "ok" : "idle"} dot label={isSelected ? "selected" : "available"} />
              ) : (
                <StatusChip status="disabled" dot label="not supported by this engine build" />
              )}
              {est && supported ? <span className="fmt-est">{est}</span> : null}
            </span>
          </button>
        );
      })}
      <style>{`
        .fmt-grid { display: grid; gap: 12px; grid-template-columns: repeat(auto-fill, minmax(230px, 1fr)); }
        .fmt-card { align-items: flex-start; background: var(--surface-2); border: 1px solid var(--line);
          border-radius: var(--r-md); cursor: pointer; display: flex; flex-direction: column; gap: 8px;
          padding: 14px; text-align: left; transition: border-color var(--dur-1) var(--ease), background var(--dur-1) var(--ease); }
        .fmt-card:hover:not(:disabled) { border-color: var(--line-strong); }
        .fmt-card.on { background: var(--accent-soft); border-color: var(--accent); }
        .fmt-card.off { cursor: not-allowed; opacity: 0.55; }
        .fmt-card:disabled { cursor: not-allowed; }
        .fmt-head { align-items: baseline; display: flex; gap: 8px; justify-content: space-between; width: 100%; }
        .fmt-name { font-size: 14px; font-weight: 600; }
        .fmt-ext { color: var(--ink-dim); font-size: 11px; }
        .fmt-purpose { color: var(--ink-mute); font-size: 12px; line-height: 1.45; }
        .fmt-foot { align-items: center; display: flex; flex-wrap: wrap; gap: 8px; justify-content: space-between; width: 100%; }
        .fmt-est { color: var(--ink-dim); font-size: 11px; font-variant-numeric: tabular-nums; }
      `}</style>
    </div>
  );
}
