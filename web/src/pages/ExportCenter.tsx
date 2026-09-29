/**
 * Export Center — dataset → filter → preview → format → prepare → verify →
 * download, with the engine as the authority for every byte that ships.
 *
 * What this page does and does not do, stated plainly because a delivery
 * console that lies about this is worse than no console:
 *
 *  - **Local and real**: the dataset is read from a file you pick (JSONL or a
 *    JSON array), previewed with bounded rows/columns, filtered, and — when
 *    you ask — written back out as canonical JSONL with a sha256 you can
 *    re-check. Nothing is uploaded; nothing is fetched from a server.
 *  - **The engine generates the exports.** `harvest export --format …` writes
 *    CSV/JSONL/XLSX/PDF/Parquet/XML/ZIP, the manifest and checksums. The page
 *    composes that exact command instead of pretending a browser can run the
 *    engine's writers.
 *  - **Verification is real**: drop a produced `manifest.json` plus its
 *    payload files and the page re-hashes them with WebCrypto, per file.
 *    `harvest verify` stays the authority for record re-counts and signatures.
 *  - **No fake progress**: state is one of a few named states; there is no
 *    animated percentage that is not measuring anything.
 */
import { useMemo, useRef, useState } from "react";
import { Button, Card, CopyButton, EmptyState, Field, MetricCard, PageHeader, SelectInput, StatusChip, TextInput } from "../components/ui";
import { Page } from "../components/AppShell";
import DataPreviewTable from "../components/DataPreviewTable";
import ExportFormatPicker, { EXPORT_FORMATS, type ExportFormatId } from "../components/ExportFormatPicker";
import { readBytes, readText, sha256Hex } from "../features/delivery/Delivery";
import { fmtBytes, fmtHash, fmtNum } from "../lib/format";
import { REDACTION_MARK } from "../lib/redaction";

/** Bounded local processing: a browser tab is not an ETL cluster. */
const MAX_RECORDS = 20_000;
const MAX_COLUMNS = 64;
const PREVIEW_ROWS = 25;
const XLSX_ROWS_PER_SHEET = 100_000;
const PARQUET_ROWS_PER_GROUP = 100_000;
const PDF_SAMPLE_ROWS = 12;

type Row = Record<string, unknown>;

interface ManifestFile {
  name: string;
  sha256: string;
  bytes: number;
}
interface ManifestDoc {
  files?: ManifestFile[];
  records?: number;
}
type VerifyState = "pending" | "match" | "mismatch" | "missing" | "no-digest";
interface VerifyRow {
  name: string;
  expected: string | null;
  actual: string | null;
  state: VerifyState;
}

/** Formats the 0.10.0 engine implements (src/export.rs). */
const AVAILABLE: readonly ExportFormatId[] = EXPORT_FORMATS.map((f) => f.id);

const SUBTLE_OK = typeof crypto !== "undefined" && !!crypto.subtle?.digest;

/** Column order: meta columns first, then payload keys, as the exporters do. */
function columnsOf(rows: readonly Row[], cap: number): string[] {
  const seen = new Set<string>();
  for (const r of rows) {
    for (const k of Object.keys(r)) if (seen.size < cap) seen.add(k);
  }
  return [...seen];
}

function estimateFor(id: ExportFormatId, rows: number, cols: number, chosen: readonly ExportFormatId[]): string {
  switch (id) {
    case "csv":
      return `${fmtNum(rows + 1)} line(s) incl. header · ${fmtNum(cols)} col`;
    case "jsonl":
      return `${fmtNum(rows)} record(s)`;
    case "xlsx":
      return `${fmtNum(Math.max(1, Math.ceil(rows / XLSX_ROWS_PER_SHEET)))} sheet(s) · ${fmtNum(rows + 1)} row(s)`;
    case "pdf":
      return `1 report · ≤ ${PDF_SAMPLE_ROWS} sample row(s)`;
    case "parquet":
      return `${fmtNum(cols)} typed col · ${fmtNum(Math.max(1, Math.ceil(rows / PARQUET_ROWS_PER_GROUP)))} row group(s)`;
    case "xml":
      return `${fmtNum(rows)} <record> element(s)`;
    case "zip":
      return `${fmtNum(chosen.filter((f) => f !== "zip").length + 2)} member(s)`;
    default:
      return "";
  }
}

export default function ExportCenter() {
  const [rows, setRows] = useState<Row[]>([]);
  const [sourceName, setSourceName] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [sourceError, setSourceError] = useState<Error | null>(null);
  const [skipped, setSkipped] = useState(0);

  const [target, setTarget] = useState("");
  const [search, setSearch] = useState("");
  const [rowCap, setRowCap] = useState(1000);
  const [hidden, setHidden] = useState<string[]>([]);

  const [selected, setSelected] = useState<ExportFormatId[]>(["csv", "jsonl"]);
  const [datasetId, setDatasetId] = useState("demo-run");
  const [outDir, setOutDir] = useState("./delivery");

  const [prepared, setPrepared] = useState<{ name: string; bytes: number; sha256: string; url: string } | null>(null);
  const [busy, setBusy] = useState(false);

  const [manifest, setManifest] = useState<ManifestDoc | null>(null);
  const [manifestName, setManifestName] = useState<string | null>(null);
  const [payloads, setPayloads] = useState<File[]>([]);
  const [verifyRows, setVerifyRows] = useState<VerifyRow[] | null>(null);
  const [verifying, setVerifying] = useState(false);
  const [verifyError, setVerifyError] = useState<string | null>(null);
  const payloadRef = useRef<HTMLInputElement | null>(null);

  const columns = useMemo(() => columnsOf(rows, MAX_COLUMNS), [rows]);
  const targets = useMemo(() => {
    const set = new Set<string>();
    for (const r of rows) {
      const t = r.target;
      if (typeof t === "string" && t.length > 0) set.add(t);
    }
    return [...set].sort();
  }, [rows]);

  const visibleColumns = useMemo(() => columns.filter((c) => !hidden.includes(c)), [columns, hidden]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    const out: Row[] = [];
    for (const r of rows) {
      if (target && r.target !== target) continue;
      if (needle && !JSON.stringify(r).toLowerCase().includes(needle)) continue;
      out.push(r);
      if (out.length >= rowCap) break;
    }
    return out;
  }, [rows, target, search, rowCap]);

  async function loadSource(file: File) {
    setLoading(true);
    setSourceError(null);
    setPrepared(null);
    setVerifyRows(null);
    try {
      const text = await readText(file);
      const parsed: Row[] = [];
      let bad = 0;
      const trimmed = text.trimStart();
      if (trimmed.startsWith("[")) {
        const arr = JSON.parse(text) as unknown;
        if (!Array.isArray(arr)) throw new Error("top-level JSON array expected");
        for (const item of arr) {
          if (item && typeof item === "object" && !Array.isArray(item)) parsed.push(item as Row);
          else bad += 1;
        }
      } else {
        for (const line of text.split("\n")) {
          const l = line.trim();
          if (!l) continue;
          try {
            const v = JSON.parse(l) as unknown;
            if (v && typeof v === "object" && !Array.isArray(v)) parsed.push(v as Row);
            else bad += 1;
          } catch {
            bad += 1;
          }
        }
      }
      if (parsed.length === 0) throw new Error(`no JSON records found in ${file.name}`);
      setRows(parsed.slice(0, MAX_RECORDS));
      setSkipped(bad + Math.max(0, parsed.length - MAX_RECORDS));
      setSourceName(file.name);
    } catch (e) {
      setRows([]);
      setSourceName(null);
      setSourceError(e instanceof Error ? e : new Error(String(e)));
    } finally {
      setLoading(false);
    }
  }

  function toggleFormat(id: ExportFormatId) {
    setSelected((prev) => (prev.includes(id) ? prev.filter((f) => f !== id) : [...prev, id]));
  }

  function command(): string {
    const from = sourceName ?? "records.jsonl";
    return [
      "harvest export",
      `--from ${from}`,
      `--to ${outDir}`,
      `--format ${selected.join(",")}`,
      datasetId.trim() ? `--dataset ${datasetId.trim()}` : "",
    ]
      .filter(Boolean)
      .join(" ");
  }

  /** Real, local work: canonical JSONL of the filtered selection + its digest. */
  async function prepare() {
    if (filtered.length === 0 || !SUBTLE_OK) return;
    setBusy(true);
    try {
      const text = filtered.map((r) => JSON.stringify(r)).join("\n") + "\n";
      const digest = await sha256Hex(new TextEncoder().encode(text).buffer as ArrayBuffer);
      const blob = new Blob([text], { type: "application/x-ndjson" });
      if (prepared) URL.revokeObjectURL(prepared.url);
      setPrepared({
        name: `dataset-${datasetId.trim() || "records"}.jsonl`,
        bytes: blob.size,
        sha256: digest,
        url: URL.createObjectURL(blob),
      });
    } catch (e) {
      setVerifyError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  async function readManifest(file: File) {
    setVerifyRows(null);
    try {
      const doc = JSON.parse(await readText(file)) as ManifestDoc;
      if (!Array.isArray(doc.files)) throw new Error("no `files` array — not a harvest export manifest");
      setManifest(doc);
      setManifestName(file.name);
    } catch (e) {
      setManifest(null);
      setManifestName(null);
      setVerifyError(e instanceof Error ? e.message : String(e));
    }
  }

  async function verify() {
    if (!manifest || !SUBTLE_OK) return;
    setVerifying(true);
    setVerifyError(null);
    try {
      const byName = new Map(payloads.map((f) => [f.name, f]));
      const out: VerifyRow[] = (manifest.files ?? []).map((f) => {
        if (!byName.has(f.name)) return { name: f.name, expected: f.sha256 ?? null, actual: null, state: "missing" };
        if (typeof f.sha256 !== "string" || f.sha256.length !== 64) {
          return { name: f.name, expected: f.sha256 ?? null, actual: null, state: "no-digest" };
        }
        return { name: f.name, expected: f.sha256, actual: null, state: "pending" };
      });
      for (const row of out) {
        if (row.state !== "pending") continue;
        const file = byName.get(row.name);
        if (!file) continue;
        const got = await sha256Hex(await readBytes(file));
        row.actual = got;
        const promised = (manifest.files ?? []).find((f) => f.name === row.name);
        row.state = got === row.expected && file.size === (promised?.bytes ?? file.size) ? "match" : "mismatch";
      }
      setVerifyRows(out);
    } catch (e) {
      setVerifyError(e instanceof Error ? e.message : String(e));
    } finally {
      setVerifying(false);
    }
  }

  const state = loading
    ? "reading file"
    : sourceError
      ? "source error"
      : rows.length === 0
        ? "idle"
        : prepared
          ? "ready"
          : "loaded";

  const matched = verifyRows?.filter((r) => r.state === "match").length ?? 0;

  return (
    <Page>
      <PageHeader
        title="Export Center"
        sub="Filter and preview a dataset, choose delivery formats, prepare the export, and verify what came back — the engine CLI remains the authority for generation."
      />

      <div className="grid grid-metrics">
        <MetricCard label="Records" value={fmtNum(rows.length)} meta={sourceName ?? "no dataset loaded"} />
        <MetricCard label="In selection" value={fmtNum(filtered.length)} unit={`/ ${fmtNum(rows.length)}`} meta={`${fmtNum(visibleColumns.length)} column(s) selected`} />
        <MetricCard label="Formats" value={fmtNum(selected.length)} unit={`/ ${AVAILABLE.length}`} meta={selected.join(", ") || "none selected"} />
        <MetricCard label="State" value={<StatusChip status={sourceError ? "error" : loading ? "queued" : "ok"} dot label={state} />} meta={SUBTLE_OK ? "WebCrypto available" : "WebCrypto unavailable — no local hashing"} />
      </div>

      <Card title="Dataset">
        <div className="ec-pick">
          <label className="ec-label">
            <span>records file (JSONL or JSON array)</span>
            <input
              type="file"
              accept=".jsonl,.ndjson,.json,application/json,text/plain"
              aria-label="records file"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void loadSource(f);
              }}
            />
          </label>
          <Field label="Dataset id" help="Sanitized into dataset-&lt;id&gt;.&lt;ext&gt; file names on the engine host.">
            <TextInput aria-label="dataset id" value={datasetId} onChange={(e) => setDatasetId(e.target.value)} />
          </Field>
          <Field label="Output directory" help="Where the engine writes the delivery.">
            <TextInput aria-label="output directory" value={outDir} onChange={(e) => setOutDir(e.target.value)} />
          </Field>
        </div>
        {sourceName && !sourceError ? (
          <p className="t-caption t-muted">
            {sourceName} · {fmtNum(columns.length)} column(s){skipped > 0 ? ` · ${fmtNum(skipped)} line(s) skipped or beyond the ${fmtNum(MAX_RECORDS)} record cap` : ""}
          </p>
        ) : null}
        {sourceError ? (
          <p className="t-caption t-err" role="alert">
            source rejected: {sourceError.message}
          </p>
        ) : null}
      </Card>

      {rows.length > 0 ? (
        <>
          <Card title="Filter">
            <div className="ec-filter">
              <Field label="Target">
                <SelectInput aria-label="target filter" value={target} onChange={(e) => setTarget(e.target.value)}>
                  <option value="">all targets</option>
                  {targets.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </SelectInput>
              </Field>
              <Field label="Search" help="Substring over the serialized record.">
                <TextInput aria-label="record search" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="mousetrap" />
              </Field>
              <Field label="Row cap">
                <TextInput
                  aria-label="row cap"
                  type="number"
                  min={1}
                  value={rowCap}
                  onChange={(e) => setRowCap(Math.max(1, Number(e.target.value) || 1))}
                />
              </Field>
            </div>
            <details className="ec-cols">
              <summary>Columns ({fmtNum(visibleColumns.length)}/{fmtNum(columns.length)} shown)</summary>
              <div className="ec-colgrid">
                {columns.map((c) => (
                  <label key={c} className="ec-col">
                    <input
                      type="checkbox"
                      checked={!hidden.includes(c)}
                      onChange={(e) => setHidden((prev) => (e.target.checked ? prev.filter((x) => x !== c) : [...prev, c]))}
                    />
                    <code>{c}</code>
                  </label>
                ))}
              </div>
            </details>
          </Card>

          <Card title="Preview" actions={<span className="t-caption t-muted">bounded: {PREVIEW_ROWS} row(s)</span>}>
            <DataPreviewTable
              columns={visibleColumns}
              rows={filtered.map((r) => Object.fromEntries(visibleColumns.map((c) => [c, r[c] ?? null])))}
              maxRows={PREVIEW_ROWS}
              ariaLabel="export preview"
            />
            <p className="t-caption t-muted">
              Credential-shaped fields are displayed as {REDACTION_MARK}; the export itself carries the real values —
              masking happens on screen, never in your data.
            </p>
          </Card>

          <Card title="Formats">
            <ExportFormatPicker
              available={AVAILABLE}
              selected={selected}
              onToggle={toggleFormat}
              estimate={(id) => estimateFor(id, filtered.length, visibleColumns.length, selected)}
            />
            <div className="ec-cmd">
              <code aria-label="export command">{command()}</code>
              <CopyButton text={command()} label="Copy command" />
            </div>
            <p className="t-caption t-muted">
              Run that on the engine host: it writes the selected formats plus manifest.json and checksums.txt. This
              page never claims to have generated those files itself.
            </p>
            <div className="ec-actions">
              <Button variant="primary" onClick={() => void prepare()} disabled={busy || filtered.length === 0 || !SUBTLE_OK}>
                {busy ? "preparing…" : "Prepare filtered JSONL"}
              </Button>
              {prepared ? (
                <>
                  <a className="btn" href={prepared.url} download={prepared.name} aria-label="download prepared dataset">
                    Download {prepared.name}
                  </a>
                  <span className="t-caption t-muted">
                    {fmtBytes(prepared.bytes)} · sha256 {fmtHash(prepared.sha256, 12)}
                  </span>
                </>
              ) : null}
            </div>
            {!SUBTLE_OK ? (
              <p className="t-caption t-err">WebCrypto is unavailable in this context, so local hashing and preparation are disabled.</p>
            ) : null}
          </Card>
        </>
      ) : (
        !sourceError && <EmptyState title="No dataset loaded" body="Pick a JSONL or JSON-array export to start. Nothing is uploaded — the file is read in this tab." />
      )}

      <Card title="Verify a delivery">
        <div className="ec-pick">
          <label className="ec-label">
            <span>manifest.json</span>
            <input
              type="file"
              accept=".json,application/json"
              aria-label="manifest file"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) void readManifest(f);
              }}
            />
          </label>
          <label className="ec-label">
            <span>payload files</span>
            <input
              type="file"
              multiple
              aria-label="payload files"
              ref={payloadRef}
              onChange={(e) => {
                setPayloads(Array.from(e.target.files ?? []));
                setVerifyRows(null);
              }}
            />
          </label>
          <Button onClick={() => void verify()} disabled={!manifest || payloads.length === 0 || verifying || !SUBTLE_OK}>
            {verifying ? "hashing…" : "Verify in browser"}
          </Button>
        </div>
        {manifestName ? (
          <p className="t-caption t-muted">
            {manifestName} · {fmtNum(manifest?.files?.length ?? 0)} promised file(s) · {fmtNum(payloads.length)} selected
          </p>
        ) : null}
        {verifyError ? <p className="t-caption t-err">{verifyError}</p> : null}
        {verifyRows ? (
          <>
            <p className="t-caption">
              {fmtNum(matched)} of {fmtNum(verifyRows.length)} file(s) match their digest. Record re-counts and the
              detached signature are the CLI's job (`harvest verify`).
            </p>
            <table className="data" aria-label="verification">
              <thead>
                <tr>
                  <th>file</th>
                  <th>expected sha256</th>
                  <th>actual</th>
                  <th>state</th>
                </tr>
              </thead>
              <tbody>
                {verifyRows.map((r) => (
                  <tr key={r.name}>
                    <td>{r.name}</td>
                    <td title={r.expected ?? undefined}>{fmtHash(r.expected)}</td>
                    <td title={r.actual ?? undefined}>{r.actual ? fmtHash(r.actual) : "—"}</td>
                    <td>
                      <StatusChip
                        status={r.state === "match" ? "ok" : r.state === "missing" || r.state === "no-digest" ? "unavailable" : "error"}
                        label={r.state}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        ) : null}
      </Card>

      <style>{`
        .ec-pick { align-items: flex-end; display: flex; flex-wrap: wrap; gap: 18px; }
        .ec-label { display: flex; flex-direction: column; font-size: 12px; gap: 6px; }
        .ec-filter { display: grid; gap: 12px; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); }
        .ec-cols { margin-top: 10px; }
        .ec-cols summary { cursor: pointer; font-size: 12px; }
        .ec-colgrid { display: grid; gap: 6px; grid-template-columns: repeat(auto-fill, minmax(170px, 1fr)); margin-top: 8px; }
        .ec-col { align-items: center; display: flex; gap: 6px; font-size: 11px; }
        .ec-cmd { align-items: center; background: var(--surface-2); border: 1px solid var(--line); border-radius: var(--r-sm);
          display: flex; gap: 10px; justify-content: space-between; margin-top: 12px; padding: 10px 12px; }
        .ec-cmd code { font-size: 12px; overflow-x: auto; white-space: pre; }
        .ec-actions { align-items: center; display: flex; flex-wrap: wrap; gap: 12px; margin-top: 12px; }
      `}</style>
    </Page>
  );
}
