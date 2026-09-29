/**
 * Artifacts — what a job produced, and whether it can still be trusted.
 *
 * The API exposes artifacts per job (`GET /control/jobs` embeds them;
 * `GET /control/artifacts/:id` is the metadata route for one). There is no
 * "list every artifact" endpoint, so this page derives the list from the
 * retained jobs and says so — it never invents a second source.
 *
 * Two actions, both server-side:
 *  - **Verify** → `GET /artifacts/:id/verify`, which re-runs the manifest and
 *    checksum comparison the CLI's `harvest verify` performs, and marks every
 *    artifact of that delivery accordingly;
 *  - **Download** → `GET /artifacts/:id`, authenticated by header, refused
 *    (409) when the delivery no longer verifies, and capped by the server.
 *
 * The page never shows a filesystem path, because the server never sends one.
 */
import { useCallback, useMemo, useState } from "react";
import { Button, Card, EmptyState, ErrorState, Field, MetricCard, PageHeader, SelectInput, TextInput } from "../components/ui";
import { Page } from "../components/AppShell";
import { ArtifactList, type ArtifactRow } from "../components/patterns/ArtifactList";
import { apiDownload, apiGet, saveDownload, ApiError } from "../lib/api";
import { usePolled } from "../hooks/usePolled";
import { useToasts } from "../hooks/useToasts";
import { useRoute, navigate } from "../app/router";
import { fmtBytes, fmtNum } from "../lib/format";
import type { ControlJobsResponse } from "./ControlJobs";

export interface ArtifactInspectResponse {
  artifact: ArtifactRow;
  job: string | null;
  verification: { state: string; records?: number; files?: number; records_basis?: string } | null;
}

interface Row extends ArtifactRow {
  jobId: string;
  operation: string;
}

const ROLE_FILTERS = ["", "payload", "manifest", "checksums", "bundle", "run_output"] as const;
const STATE_FILTERS = ["", "ready", "failed"] as const;

export default function ArtifactsPage() {
  const route = useRoute();
  const want = route.query.get("artifact") ?? "";
  const toasts = useToasts();
  const [role, setRole] = useState<string>("");
  const [state, setState] = useState<string>("");
  const [search, setSearch] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);

  const jobs = usePolled<ControlJobsResponse>(
    (s) => apiGet<ControlJobsResponse>("/control/jobs", { params: { limit: 50 }, signal: s, noCache: true }),
    10_000,
  );

  const rows: Row[] = useMemo(() => {
    const out: Row[] = [];
    for (const j of jobs.data?.items ?? []) {
      for (const a of j.artifacts) {
        out.push({ ...a, job: a.job ?? j.id, jobId: j.id, operation: j.operation });
      }
    }
    return out;
  }, [jobs.data]);

  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return rows.filter(
      (r) =>
        (role === "" || r.role === role) &&
        (state === "" || r.state === state) &&
        (needle === "" ||
          r.name.toLowerCase().includes(needle) ||
          r.id.toLowerCase().includes(needle) ||
          r.sha256.toLowerCase().includes(needle)),
    );
  }, [rows, role, state, search]);

  const totals = useMemo(() => {
    let bytes = 0;
    let ready = 0;
    let failed = 0;
    for (const r of rows) {
      bytes += r.bytes;
      if (r.state === "failed") failed += 1;
      else ready += 1;
    }
    return { bytes, ready, failed };
  }, [rows]);

  const refresh = useCallback(() => jobs.refresh(), [jobs]);

  const verify = useCallback(
    async (a: ArtifactRow) => {
      setBusyId(a.id);
      try {
        const v = await apiGet<{ state: string; detail?: { records?: number; files?: number } }>(
          `/artifacts/${encodeURIComponent(a.id)}/verify`,
          { noCache: true },
        );
        const records = v.detail?.records;
        toasts.push(
          v.state === "ok" ? "ok" : "err",
          records === undefined
            ? `${a.name}: verification ${v.state}`
            : `${a.name}: verification ${v.state} — ${records} record(s) across ${v.detail?.files ?? 0} file(s)`,
        );
        refresh();
      } catch (e) {
        toasts.push("err", `${a.name}: ${e instanceof ApiError ? e.message : String(e)}`);
      } finally {
        setBusyId(null);
      }
    },
    [refresh, toasts],
  );

  const download = useCallback(
    async (a: ArtifactRow) => {
      setBusyId(a.id);
      try {
        const dl = await apiDownload(`/artifacts/${encodeURIComponent(a.id)}`);
        const name = saveDownload(dl, a.name);
        toasts.push(
          "ok",
          name === null
            ? `${a.name}: ${fmtBytes(dl.bytes.length)} received (sha256 ${dl.sha256?.slice(0, 12) ?? "—"})`
            : `${a.name}: saved ${name} (${fmtBytes(dl.bytes.length)})`,
        );
      } catch (e) {
        const ae = e instanceof ApiError ? e : null;
        toasts.push(
          "err",
          ae?.kind === "conflict"
            ? `${a.name}: the server refuses this download — the delivery no longer verifies`
            : `${a.name}: ${ae?.message ?? String(e)}`,
        );
        refresh();
      } finally {
        setBusyId(null);
      }
    },
    [refresh, toasts],
  );

  const inspect = usePolled<ArtifactInspectResponse | null>(
    (s) =>
      want
        ? apiGet<ArtifactInspectResponse>(`/control/artifacts/${encodeURIComponent(want)}`, { signal: s, noCache: true })
        : Promise.resolve(null),
    want ? 15_000 : 600_000,
    want,
  );

  return (
    <Page>
      <PageHeader
        title="Artifacts"
        sub={
          <>
            What jobs produced, verified against the manifest the server wrote. Derived from the retained job list —
            there is no separate artifact index, and none is invented here.
          </>
        }
        actions={
          <Button variant="ghost" onClick={refresh} disabled={jobs.state === "refreshing"}>
            Refresh
          </Button>
        }
      />

      <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit,minmax(180px,1fr))" }}>
        <MetricCard label="Artifacts" value={fmtNum(rows.length)} meta={`${fmtNum(filtered.length)} shown`} />
        <MetricCard label="Bytes retained" value={fmtBytes(totals.bytes)} meta="sum of registered files" />
        <MetricCard label="Ready" value={fmtNum(totals.ready)} meta="downloadable" />
        <MetricCard label="Failed verification" value={fmtNum(totals.failed)} meta="download refused" />
      </div>

      {jobs.error && <ErrorState error={jobs.error} onRetry={refresh} />}

      {want && (
        <Card title={`Artifact ${want}`} actions={<Button variant="ghost" onClick={() => navigate("/artifacts")}>Clear</Button>}>
          {inspect.error ? (
            <ErrorState error={inspect.error} />
          ) : inspect.data ? (
            <div style={{ display: "grid", gap: 6 }}>
              <div>
                <span className="t-caption">name</span> <code className="t-code">{inspect.data.artifact.name}</code>
              </div>
              <div>
                <span className="t-caption">role</span> {inspect.data.artifact.role} ·{" "}
                <span className="t-caption">job</span> {inspect.data.job ?? "—"}
              </div>
              <div>
                <span className="t-caption">sha256</span>{" "}
                <code className="t-code" style={{ wordBreak: "break-all" }}>
                  {inspect.data.artifact.sha256}
                </code>
              </div>
              <div className="t-caption">
                {fmtBytes(inspect.data.artifact.bytes)} · {inspect.data.artifact.content_type} · state{" "}
                {inspect.data.artifact.state}
              </div>
              {inspect.data.verification && (
                <div className="t-caption">
                  verification {inspect.data.verification.state}
                  {inspect.data.verification.records !== undefined &&
                    ` · ${fmtNum(inspect.data.verification.records)} record(s) · basis ${inspect.data.verification.records_basis ?? "not-checked"}`}
                </div>
              )}
            </div>
          ) : (
            <p className="t-caption" style={{ margin: 0 }}>
              Loading artifact metadata…
            </p>
          )}
        </Card>
      )}

      <Card title="Filters">
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end" }}>
          <Field label="Role">
            <SelectInput aria-label="Role" value={role} onChange={(e) => setRole(e.target.value)}>
              {ROLE_FILTERS.map((r) => (
                <option key={r || "any"} value={r}>
                  {r || "any"}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="State">
            <SelectInput aria-label="State" value={state} onChange={(e) => setState(e.target.value)}>
              {STATE_FILTERS.map((s) => (
                <option key={s || "any"} value={s}>
                  {s || "any"}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="Search" help="name, artifact id or sha256">
            <TextInput aria-label="Search" value={search} placeholder="records.csv" onChange={(e) => setSearch(e.target.value)} />
          </Field>
        </div>
      </Card>

      <Card title={`Artifacts (${fmtNum(filtered.length)})`}>
        {filtered.length === 0 ? (
          <EmptyState
            title="No artifacts match"
            body={
              jobs.data
                ? "Nothing has been produced yet, or the filters exclude everything. Run an export from Control Jobs and it will appear here."
                : "Loading the job list…"
            }
          />
        ) : (
          <ArtifactList
            artifacts={filtered}
            busyId={busyId}
            showJob
            onVerify={verify}
            onDownload={download}
          />
        )}
      </Card>

      <Card title="How downloads stay safe">
        <ul className="t-caption" style={{ margin: 0, paddingLeft: 18, display: "grid", gap: 4 }}>
          <li>
            The token travels in an <code className="t-code">authorization</code> header, never in the URL — so it
            cannot leak through a link, a referer or an access log.
          </li>
          <li>
            Ids are validated by the server: an id that is not <code className="t-code">art-&lt;n&gt;</code> is a 404,
            so no path can be walked out of the delivery directory.
          </li>
          <li>
            A delivery that fails verification stops being downloadable (409) until it is re-exported. The console
            disables the button, but the server is the one that refuses.
          </li>
          <li>
            Size is bounded server-side before a single byte is read; an artifact the registry does not know about
            cannot be requested at all.
          </li>
        </ul>
      </Card>
    </Page>
  );
}
