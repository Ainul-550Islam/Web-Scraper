/**
 * Control Job Detail — one control-plane job, exactly as the server describes
 * it: `/control/job?job=<job-id>`.
 *
 * Everything on the page comes from `GET /control/jobs/:id`. Where the server
 * withholds something (a filesystem path, a request payload, a provider
 * credential), the page is empty in that spot by design — there is no client
 * side reconstruction of anything the API does not send.
 *
 * The page polls while the job can still change (queued/running) and stops
 * once it is terminal, so a finished job costs the server nothing.
 */
import { useCallback, useMemo, useState } from "react";
import { Button, Card, CopyButton, EmptyState, ErrorState, MetricCard, PageHeader, StatusChip } from "../components/ui";
import { Page } from "../components/AppShell";
import { ArtifactList, type ArtifactRow } from "../components/patterns/ArtifactList";
import { JobStateBadge } from "../components/patterns/JobStateBadge";
import { canCancel, isTerminal, jobBlurb } from "../components/patterns/JobState.model";
import { apiDownload, apiGet, apiPost, saveDownload, ApiError } from "../lib/api";
import { usePolled } from "../hooks/usePolled";
import { useToasts } from "../hooks/useToasts";
import { useRoute, navigate } from "../app/router";
import { fmtBytes, fmtDateTime, fmtNum } from "../lib/format";
import type { ControlJob } from "./ControlJobs";

export interface VerifyResponse {
  state: string;
  delivery?: string | null;
  detail?: {
    state?: string;
    records?: number;
    files?: number;
    records_basis?: string;
  } | null;
}

const RESULT_KEYS_SHOWN = new Set(["id", "state", "operation"]);


export default function ControlJobDetail() {
  const route = useRoute();
  const jobId = route.query.get("job") ?? "";
  const toasts = useToasts();
  const [busyId, setBusyId] = useState<string | null>(null);
  // Poll every 2 s while the job can still change; once it is terminal the
  // interval stretches to 10 minutes, so a finished job costs the server
  // (almost) nothing and the page still parks on the final record.
  const poller = usePolled<ControlJob>(
    (s) => apiGet<ControlJob>(`/control/jobs/${encodeURIComponent(jobId)}`, { signal: s, noCache: true }),
    (data) => data && isTerminal(data.state) ? 600_000 : 2_000,
    jobId,
  );
  const detail = poller.data;
  const error = poller.error;


  const verify = useCallback(
    async (a: ArtifactRow) => {
      setBusyId(a.id);
      try {
        const v = await apiGet<VerifyResponse>(`/artifacts/${encodeURIComponent(a.id)}/verify`, { noCache: true });
        const records = v.detail?.records;
        toasts.push(
          v.state === "ok" ? "ok" : "err",
          records === undefined
            ? `${a.name}: verification ${v.state}`
            : `${a.name}: verification ${v.state} — ${records} record(s) across ${v.detail?.files ?? 0} file(s)`,
        );
        poller.refresh();
      } catch (e) {
        toasts.push("err", `${a.name}: ${e instanceof ApiError ? e.message : String(e)}`);
      } finally {
        setBusyId(null);
      }
    },
    [poller, toasts],
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
            ? `${a.name}: the server refuses this download — the delivery failed verification`
            : `${a.name}: ${ae?.message ?? String(e)}`,
        );
        poller.refresh();
      } finally {
        setBusyId(null);
      }
    },
    [poller, toasts],
  );

  const cancel = useCallback(async () => {
    if (!detail) return;
    setBusyId(detail.id);
    try {
      await apiPost<ControlJob>(`/control/jobs/${encodeURIComponent(detail.id)}/cancel`, {});
      toasts.push("ok", `${detail.id}: cancel accepted`);
      poller.refresh();
    } catch (e) {
      const ae = e instanceof ApiError ? e : null;
      toasts.push(
        ae?.kind === "conflict" ? "info" : "err",
        ae?.kind === "conflict" ? `${detail.id} already finished — nothing to cancel` : `${detail.id}: ${ae?.message ?? String(e)}`,
      );
    } finally {
      setBusyId(null);
    }
  }, [detail, poller, toasts]);

  const counters = detail?.counters;
  const resultEntries = useMemo(() => {
    const r = detail?.result;
    if (!r || typeof r !== "object") return [] as [string, unknown][];
    return Object.entries(r).filter(([k]) => !RESULT_KEYS_SHOWN.has(k));
  }, [detail]);

  if (!jobId) {
    return (
      <Page>
        <PageHeader title="Control Job" sub="no job selected" />
        <EmptyState title="Pick a job" body="Open a job from Control Jobs, or add ?job=<id> to this address." />
      </Page>
    );
  }

  if (error && !detail) {
    return (
      <Page>
        <PageHeader title={`Control Job ${jobId}`} sub="the server did not return this job" />
        <ErrorState error={error} onRetry={() => poller.refresh()} />
        <p className="t-caption">
          A 404 here means the job id is unknown or has aged out of the retained window — not that it never ran.
        </p>
      </Page>
    );
  }

  return (
    <Page>
      <PageHeader
        title={`Job ${jobId}`}
        sub={detail ? `${detail.operation} · ${jobBlurb(detail.state)}` : "loading…"}
        actions={
          <>
            <Button variant="ghost" onClick={() => poller.refresh()} disabled={poller.state === "refreshing"}>
              Refresh
            </Button>
            <Button
              variant="ghost"
              disabled={!detail || !canCancel(detail.state) || busyId === detail.id}
              onClick={cancel}
              title={detail && canCancel(detail.state) ? "ask the server to cancel this job" : "this job is terminal"}
            >
              Cancel job
            </Button>
            <Button variant="ghost" onClick={() => navigate("/control")}>
              All jobs
            </Button>
          </>
        }
      />

      {detail && (
        <>
          <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit,minmax(180px,1fr))" }}>
            <MetricCard label="State" value={<JobStateBadge state={detail.state} cancelRequested={detail.cancel_requested} />} meta={jobBlurb(detail.state)} />
            <MetricCard label="Operation" value={detail.operation} meta={detail.resource} />
            <MetricCard
              label="Pages / records"
              value={`${fmtNum(counters?.pages_fetched ?? 0)} / ${fmtNum(counters?.records_written ?? 0)}`}
              meta={`${fmtNum(counters?.records_extracted ?? 0)} extracted`}
            />
            <MetricCard
              label="Failures"
              value={fmtNum(counters?.failures ?? 0)}
              meta={`${fmtNum(counters?.retries ?? 0)} retried · ${fmtNum(counters?.quarantined ?? 0)} quarantined`}
            />
            <MetricCard label="Started" value={fmtDateTime(detail.created_ts)} meta={`deadline ${fmtDateTime(detail.deadline_ts)}`} />
            <MetricCard label="Artifacts" value={fmtNum(detail.artifacts.length)} meta={detail.artifacts.length > 0 ? "see below" : "none registered"} />
          </div>

          {detail.error && (
            <Card title="Why it did not succeed">
              <p style={{ margin: 0, display: "flex", gap: 8, alignItems: "center" }}>
                <StatusChip status="failed" /> {detail.error}
              </p>
            </Card>
          )}

          <Card title="Limits the server applied">
            <div style={{ display: "grid", gap: 6, gridTemplateColumns: "repeat(auto-fit,minmax(160px,1fr))" }}>
              {Object.entries(detail.limits).map(([k, v]) => (
                <div key={k}>
                  <div className="t-caption">{k}</div>
                  <code className="t-code">{fmtNum(Number(v))}</code>
                </div>
              ))}
            </div>
            <p className="t-caption" style={{ marginTop: 8 }}>
              These are the bounds the job ran under — already clamped to the attached configuration. Asking for more
              does not widen them.
            </p>
          </Card>

          <Card title={`Artifacts (${detail.artifacts.length})`}>
            <ArtifactList artifacts={detail.artifacts} busyId={busyId} onVerify={verify} onDownload={download} />
            {detail.artifacts.length === 0 && (
              <p className="t-caption" style={{ marginTop: 8 }}>
                {detail.operation === "ai"
                  ? "An AI round produces metadata only — there is nothing to download, by design."
                  : "A failed job registers nothing: the server will not offer a partial delivery."}
              </p>
            )}
          </Card>

          {resultEntries.length > 0 && (
            <Card title="Result">
              <div style={{ display: "grid", gap: 6, gridTemplateColumns: "repeat(auto-fit,minmax(180px,1fr))" }}>
                {resultEntries.map(([k, v]) => (
                  <div key={k} style={{ minWidth: 0 }}>
                    <div className="t-caption">{k}</div>
                    <code className="t-code" style={{ wordBreak: "break-all" }}>
                      {typeof v === "object" && v !== null ? JSON.stringify(v) : String(v)}
                    </code>
                  </div>
                ))}
              </div>
            </Card>
          )}

          <Card title="Timeline">
            <div style={{ display: "grid", gap: 4 }}>
              {detail.events.map((e, i) => (
                <div key={`${e.ts}-${i}`} style={{ display: "flex", gap: 10, alignItems: "baseline" }}>
                  <span className="t-caption" style={{ minWidth: 160 }}>
                    {fmtDateTime(e.ts)}
                  </span>
                  <StatusChip status={e.kind === "failed" ? "failed" : e.kind} dot={false} />
                  <span className="t-caption">
                    {e.detail && Object.keys(e.detail).length > 0 ? JSON.stringify(e.detail) : ""}
                  </span>
                </div>
              ))}
            </div>
          </Card>

          <Card title="Audit reference">
            <p style={{ margin: 0, display: "flex", gap: 8, alignItems: "center" }}>
              <code className="t-code">{detail.audit_ref}</code>
              <CopyButton text={detail.audit_ref} label="Copy audit ref" />
              <span className="t-caption">
                the server's audit ledger records submit/start/finish (and any refusal) for this job — the console does
                not keep its own copy.
              </span>
            </p>
          </Card>
        </>
      )}
    </Page>
  );
}
