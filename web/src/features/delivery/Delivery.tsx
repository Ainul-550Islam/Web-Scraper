/**
 * Delivery — verify an export folder in the browser.
 *
 * The engine's manifest (`manifest.json`) promises a sha256 per payload
 * file. This page re-hashes whatever files you select and compares: a fast
 * sanity check after a download or copy, using ONLY the documented manifest
 * fields (identity.records, identity.generated_ts, files[].name/sha256/
 * bytes — `src/artifact.rs` in the Rust crate is the schema owner).
 *
 * Honest scope, stated on the page: this is a convenience check of digests
 * and byte sizes. `harvest verify DIR --key KEY` remains the authority — it
 * additionally re-counts record lines against the manifest and validates the
 * detached Ed25519 signature, neither of which belongs in a browser.
 *
 * crypto.subtle exists only in secure contexts (https or localhost); over a
 * plain http LAN address the page says so instead of pretending success.
 * No bytes ever leave the page — everything is read via File objects.
 */
import { useMemo, useRef, useState } from "react";
import { Button, Card, EmptyState, PageHeader, StatusChip } from "../../components/ui";
import { fmtBytes, fmtHash, fmtNum } from "../../lib/format";
import { pctOf } from "../../lib/dataFormatters";
import { Page } from "../../components/AppShell";

interface ManifestFile {
  name: string;
  sha256: string;
  bytes: number;
  lines?: number;
}
interface ManifestDoc {
  identity?: { records?: number; generated_ts?: number };
  files?: ManifestFile[];
}

type RowState = "pending" | "match" | "mismatch" | "missing" | "too-big" | "no-digest";
interface Row {
  name: string;
  expected: string | null;
  actual: string | null;
  bytes: number | null;
  state: RowState;
}

const MAX_BYTES = 64 * 1024 * 1024; // refuse to hash anything larger, honestly

// Blob.arrayBuffer/.text exist in every modern browser, but not in every
// jsdom; FileReader is the fallback that keeps this page testable exactly
// as shipped (and works in browsers too — same underlying bytes).
// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function readBytes(file: File): Promise<ArrayBuffer> {
  if (typeof file.arrayBuffer === "function") return file.arrayBuffer();
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(r.result as ArrayBuffer);
    r.onerror = () => reject(r.error ?? new Error("read failed"));
    r.readAsArrayBuffer(file);
  });
}

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function readText(file: File): Promise<string> {
  if (typeof file.text === "function") return file.text();
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(r.error ?? new Error("read failed"));
    r.readAsText(file);
  });
}

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export async function sha256Hex(buf: ArrayBuffer): Promise<string> {
  // re-view through this realm's Uint8Array: FileReader results can be
  // foreign-realm buffers (jsdom), and SubtleCrypto is picky about that
  const dig = await crypto.subtle.digest("SHA-256", new Uint8Array(buf));
  return [...new Uint8Array(dig)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** Pure pre-hash plan: which manifest entries have a picked file, which are
 *  missing, and which lack a 64-hex digest the browser can trust. */
function planRows(manifest: ManifestDoc, pickedNames: Set<string>): Row[] {
  return (manifest.files ?? []).map((f) => {
    if (!pickedNames.has(f.name))
      return { name: f.name, expected: f.sha256 ?? null, actual: null, bytes: null, state: "missing" as const };
    if (typeof f.sha256 !== "string" || f.sha256.length !== 64)
      return { name: f.name, expected: f.sha256 ?? null, actual: null, bytes: null, state: "no-digest" as const };
    return { name: f.name, expected: f.sha256, actual: null, bytes: null, state: "pending" as const };
  });
}

export default function Delivery() {
  const [manifest, setManifest] = useState<ManifestDoc | null>(null);
  const [manifestName, setManifestName] = useState<string | null>(null);
  const [manifestError, setManifestError] = useState<string | null>(null);
  const [payloadFiles, setPayloadFiles] = useState<File[]>([]);
  const [rows, setRows] = useState<Row[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const payloadRef = useRef<HTMLInputElement | null>(null);
  const subtleOk = typeof crypto !== "undefined" && !!crypto.subtle?.digest;

  async function readManifest(file: File) {
    setManifestError(null);
    setRows(null);
    try {
      const doc = JSON.parse(await readText(file)) as ManifestDoc;
      if (typeof doc !== "object" || doc === null || !Array.isArray(doc.files)) {
        throw new Error("no `files` array — not a harvest export manifest");
      }
      setManifest(doc);
      setManifestName(file.name);
    } catch (e) {
      setManifest(null);
      setManifestName(null);
      setManifestError(e instanceof Error ? e.message : String(e));
    }
  }

  async function verify() {
    if (!manifest || !subtleOk) return;
    setBusy(true);
    setNote(null);
    try {
      const byName = new Map(payloadFiles.map((f) => [f.name, f]));
      const planned = planRows(manifest, new Set(byName.keys()));
      for (const row of planned) {
        if (row.state !== "pending") continue;
        const file = byName.get(row.name)!;
        if (file.size > MAX_BYTES) {
          row.state = "too-big";
          row.bytes = file.size;
          continue;
        }
        row.bytes = file.size;
        const got = await sha256Hex(await readBytes(file));
        row.actual = got;
        const promised = manifest.files?.find((f) => f.name === row.name);
        const bytesOk = promised === undefined || promised.bytes === file.size;
        row.state = got === row.expected && bytesOk ? "match" : "mismatch";
      }
      setRows(planned);
      if (payloadFiles.length > new Set(manifest.files?.map((f) => f.name) ?? []).size) {
        setNote("some selected files are not in the manifest (extras are the manifest's siblings, not violations)");
      }
    } catch (e) {
      setNote(`hashing failed: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      setBusy(false);
    }
  }

  const matched = useMemo(() => rows?.filter((r) => r.state === "match").length ?? 0, [rows]);
  const total = rows?.length ?? 0;
  const canVerify = manifest !== null && payloadFiles.length > 0 && subtleOk && !busy;

  return (
    <Page>
      <PageHeader
        title="Delivery Verify"
        sub="Re-hash export payload against manifest.json locally — the CLI `harvest verify` (line counts + signature) stays the authority."
      />
      <Card title="Files">
        <div className="dv-pick">
          <label className="dv-label">
            <span>manifest.json</span>
            <input
              type="file"
              accept=".json,application/json"
              aria-label="manifest file"
              onChange={(e) => {
                const mf = e.target.files?.[0];
                if (mf) void readManifest(mf);
              }}
            />
          </label>
          <label className="dv-label">
            <span>payload files (the manifest's siblings)</span>
            <input
              type="file"
              multiple
              aria-label="payload files"
              ref={payloadRef}
              onChange={(e) => {
                setPayloadFiles(Array.from(e.target.files ?? []));
                setRows(null);
              }}
            />
          </label>
          <Button onClick={() => void verify()} disabled={!canVerify}>
            {busy ? "hashing…" : "Verify in browser"}
          </Button>
        </div>
        {manifestName && (
          <p className="t-caption t-muted">
            manifest: {manifestName} · {fmtNum(manifest?.files?.length ?? 0)} promised file(s) ·{" "}
            {fmtNum(payloadFiles.length)} selected
          </p>
        )}
        {manifestError && <p className="t-caption t-err">manifest rejected: {manifestError}</p>}
        {note && <p className="t-caption t-muted">note: {note}</p>}
        {!subtleOk && (
          <EmptyState
            title="WebCrypto unavailable here"
            body="crypto.subtle only runs in secure contexts (https or localhost). Over a plain http address, use `harvest verify` on the machine that holds the export."
          />
        )}
        {subtleOk && !manifest && !manifestError && (
          <EmptyState
            title="No manifest loaded"
            body="Select a manifest.json and the payload files next to it, then Verify. Nothing is uploaded — digests are computed in this tab."
          />
        )}
        {rows && (
          <>
            <p className="t-caption">
              {fmtNum(matched)} of {fmtNum(total)} manifest file(s) match their digest
              {total > 0 && ` (${pctOf(matched, total).toFixed(0)}%)`}; record line counts and the
              detached signature are not checked here — that is the CLI's job.
            </p>
            <table className="data">
              <thead>
                <tr>
                  <th>file</th>
                  <th>bytes</th>
                  <th>expected sha256</th>
                  <th>actual</th>
                  <th>state</th>
                </tr>
              </thead>
              <tbody>
                {rows.map((r) => (
                  <tr key={r.name}>
                    <td>{r.name}</td>
                    <td>{r.bytes === null ? "—" : fmtBytes(r.bytes)}</td>
                    <td title={r.expected ?? undefined}>{fmtHash(r.expected)}</td>
                    <td title={r.actual ?? undefined}>{r.actual ? fmtHash(r.actual) : "—"}</td>
                    <td>
                      <StatusChip
                        status={r.state === "match" ? "ok" : r.state === "missing" || r.state === "too-big" ? "unavailable" : r.state === "pending" ? "queued" : "error"}
                        label={r.state}
                      />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
        {manifest?.identity !== undefined && (
          <p className="t-caption t-muted">
            manifest identity: {fmtNum(manifest.identity.records ?? 0)} record(s)
            {typeof manifest.identity.generated_ts === "number"
              ? ` · generated ${new Date(manifest.identity.generated_ts * 1000).toISOString()}`
              : ""}
          </p>
        )}
        <style>{`
          .dv-pick { align-items: flex-end; display: flex; gap: 18px; flex-wrap: wrap; }
          .dv-label { display: flex; flex-direction: column; font-size: 12px; gap: 6px; }
        `}</style>
      </Card>
    </Page>
  );
}
