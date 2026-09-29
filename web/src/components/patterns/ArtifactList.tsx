// ArtifactList — one row per registered artifact, with the two actions the
// API actually offers (verify, download) and nothing invented around them.
//
// Everything shown comes from the registry: name, role, bytes, sha256, state.
// There is deliberately no path column — the API never sends one, and a client
// that learned the server's directory layout would be a client that could be
// tricked into asking for something outside the delivery.

import { Button, CopyButton, StatusChip } from "../ui";
import { fmtBytes, fmtHash, fmtNum } from "../../lib/format";

export type ArtifactRole = "payload" | "manifest" | "checksums" | "bundle" | "run_output";

export interface ArtifactRow {
  id: string;
  job: string | null;
  name: string;
  role: string;
  bytes: number;
  sha256: string;
  content_type: string;
  state: string;
  downloadable: boolean;
  created_ts: number;
  verification?: {
    state: string;
    records?: number;
    files?: number;
    records_basis?: string;
  } | null;
}

// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export const ARTIFACT_ROLES: ArtifactRole[] = [
  "payload",
  "manifest",
  "checksums",
  "bundle",
  "run_output",
];

/** Roles that participate in a manifest-verified delivery. */
// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function isDeliveryRole(role: string): boolean {
  return role !== "run_output";
}

export function ArtifactList({
  artifacts,
  busyId = null,
  onVerify,
  onDownload,
  showJob = false,
}: {
  artifacts: ArtifactRow[];
  busyId?: string | null;
  onVerify?: (a: ArtifactRow) => void;
  onDownload?: (a: ArtifactRow) => void;
  showJob?: boolean;
}) {
  if (artifacts.length === 0) {
    return (
      <p className="t-caption" style={{ margin: 0 }}>
        No artifacts registered yet.
      </p>
    );
  }
  return (
    <div style={{ display: "grid", gap: 8 }}>
      {artifacts.map((a) => {
        const busy = busyId === a.id;
        const verification = a.verification?.state ?? null;
        return (
          <div
            key={a.id}
            data-testid="artifact-row"
            style={{
              display: "grid",
              gridTemplateColumns: "minmax(0,1fr) auto",
              gap: 8,
              alignItems: "center",
              padding: "8px 0",
              borderTop: "1px solid var(--line)",
            }}
          >
            <div style={{ minWidth: 0 }}>
              <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
                <code className="t-code">{a.name}</code>
                <span className="chip">{a.role}</span>
                <StatusChip status={a.state} />
                {verification && verification !== a.state && (
                  <span className="t-caption">delivery: {verification}</span>
                )}
                {showJob && a.job && <span className="t-caption">{a.job}</span>}
              </div>
              <div className="t-caption" style={{ marginTop: 2 }}>
                {fmtBytes(a.bytes)} · {a.content_type} · sha256{" "}
                <code className="t-code">{fmtHash(a.sha256)}</code>{" "}
                <CopyButton text={a.sha256} label="Copy sha256" />
              </div>
              {a.verification?.records !== undefined && (
                <div className="t-caption">
                  {fmtNum(a.verification.records)} record(s) across{" "}
                  {fmtNum(a.verification.files ?? 0)} file(s) · basis{" "}
                  {a.verification.records_basis ?? "not-checked"}
                </div>
              )}
            </div>
            <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
              {onVerify && isDeliveryRole(a.role) && (
                <Button variant="ghost" className="sm" disabled={busy} onClick={() => onVerify(a)}>
                  {busy ? "Working…" : "Verify"}
                </Button>
              )}
              {onDownload && (
                <Button
                  variant="ghost"
                  className="sm"
                  disabled={busy || !a.downloadable}
                  title={a.downloadable ? "download the bytes" : "this artifact is not downloadable"}
                  onClick={() => onDownload(a)}
                >
                  Download
                </Button>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export default ArtifactList;
