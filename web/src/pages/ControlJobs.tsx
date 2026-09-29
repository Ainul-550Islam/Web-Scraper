/**
 * Control Jobs — the console end of the authenticated control plane.
 *
 * What this page is: a client for `POST /control/{crawl,export,ai}`,
 * `GET /control/jobs`, `GET /control/jobs/:id` and
 * `POST /control/jobs/:id/cancel`. Every state, counter, ceiling and verdict
 * it paints came from the server; nothing is simulated, and a job that has not
 * reported yet is shown as `queued`/`running`, never as a percentage.
 *
 * What it is not: it does not execute anything itself. The engine runs the
 * crawl, the exporter renders the formats, the provider answers the AI round,
 * and the admin server owns authentication, validation, concurrency, the
 * deadline and cancellation. This page submits one bounded command and then
 * reports what the server says about it.
 *
 * Safety properties the page keeps visible:
 *  - submissions carry the operator's bearer token (never a URL parameter) and
 *    an `Idempotency-Key`, so a double-click cannot start two jobs;
 *  - the concurrency/queue ceilings are shown, not assumed — a 503 from a full
 *    queue is displayed as that, not as a failure of the console;
 *  - cancellation is per job and authorized like any other request; a job that
 *    already finished answers 409 and the page says so.
 */
import { useCallback, useMemo, useState } from "react";
import { Button, Card, EmptyState, ErrorState, Field, MetricCard, PageHeader, SelectInput, StatusChip } from "../components/ui";
import { Page } from "../components/AppShell";
import { ArtifactList, type ArtifactRow } from "../components/patterns/ArtifactList";
import { CommandComposer, emptyDraft, buildCommand, validateCommand, type CommandDraft, CEILINGS } from "../components/patterns/CommandComposer";
import { JobStateBadge, JobStateLegend, canCancel, isTerminal } from "../components/patterns/JobStateBadge";
import { apiGet, apiPost, commandIdempotencyKey, ApiError } from "../lib/api";
import { usePolled } from "../hooks/usePolled";
import { useToasts } from "../hooks/useToasts";
import { navigate } from "../app/router";
import type { ConfigDocument } from "../lib/types";
import { fmtDateTime, fmtNum } from "../lib/format";

/* ── response shapes (src/execution.rs) ─────────────────────────────── */

export interface ControlJob {
  id: string;
  operation: string;
  state: string;
  resource: string;
  created_ts: number;
  updated_ts: number;
  deadline_ts: number;
  cancel_requested: boolean;
  limits: {
    max_pages: number;
    max_depth: number;
    max_records: number;
    max_export_bytes: number;
    max_ai_input_bytes: number;
    timeout_secs: number;
  };
  counters: {
    pages_fetched: number;
    records_extracted: number;
    records_written: number;
    requests_total: number;
    failures: number;
    retries: number;
    quarantined: number;
    blocked: number;
    deduped: number;
    schema_drifts: number;
  };
  artifacts: ArtifactRow[];
  result: Record<string, unknown> | null;
  error: string | null;
  audit_ref: string;
  events: { ts: number; kind: string; detail?: Record<string, unknown> }[];
  /** Present on the submit reply: true when an Idempotency-Key replayed. */
  replayed?: boolean;
}

export interface ControlJobsResponse {
  items: ControlJob[];
  total: number;
  in_flight?: number;
  caps?: { max_concurrent_jobs: number; max_queue_depth: number };
  limit?: number;
  offset?: number;
}

const STATE_FILTERS = ["", "queued", "running", "completed", "failed", "cancelled"] as const;
const OP_FILTERS = ["", "crawl", "export", "ai"] as const;

export default function ControlJobs() {
  const toasts = useToasts();
  const [stateFilter, setStateFilter] = useState<string>("");
  const [opFilter, setOpFilter] = useState<string>("");
  const [limit, setLimit] = useState(25);
  const [offset, setOffset] = useState(0);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const config = usePolled<ConfigDocument>((s) => apiGet<ConfigDocument>("/config", { signal: s }), 30_000);
  const targetIds = useMemo(
    () => (config.data?.targets ?? []).map((t) => t.id).filter((id): id is string => typeof id === "string" && id.length > 0),
    [config.data],
  );

  const query = useMemo(() => {
    const q: Record<string, string> = { limit: String(limit), offset: String(offset) };
    if (stateFilter) q.state = stateFilter;
    if (opFilter) q.op = opFilter;
    return q;
  }, [stateFilter, opFilter, limit, offset]);

  // 2 s while anything is in flight, 15 s when the queue is quiet: the page
  // never polls faster than the server's own job pump.
  // `key` is a primitive on purpose: usePolled re-arms its interval whenever
  // the key changes identity, so passing an array here would re-fetch on every
  // render (an accidental hot loop against the engine).
  const jobs = usePolled<ControlJobsResponse>(
    (s) => apiGet<ControlJobsResponse>("/control/jobs", { params: query, signal: s, noCache: true }),
    2_000,
    `${stateFilter}|${opFilter}|${limit}|${offset}`,
  );

  const [draft, setDraft] = useState<CommandDraft>(() => emptyDraft("export"));
  const issues = useMemo(() => validateCommand(draft, targetIds), [draft, targetIds]);

  const refresh = useCallback(() => {
    jobs.refresh?.();
  }, [jobs]);

  const submit = useCallback(
    async (body: Record<string, unknown>) => {
      setSubmitting(true);
      setSubmitError(null);
      const op = String(body.op ?? "");
      // the route is one of three known strings — never assembled from input
      const route = op === "crawl" ? "/control/crawl" : op === "export" ? "/control/export" : "/control/ai";
      const key = commandIdempotencyKey(op, body);
      try {
        const job = await apiPost<ControlJob>(route, body, { idempotencyKey: key });
        toasts.push("ok", `${op} accepted as ${job.id}${job.replayed === true ? " (replayed submission)" : ""}`);
        refresh();
        navigate(`/control/job?job=${encodeURIComponent(job.id)}`);
      } catch (e) {
        const ae = e instanceof ApiError ? e : null;
        const message =
          ae === null
            ? String(e)
            : ae.kind === "unauthorized"
              ? "the server refused the command: set a token on the connect screen"
              : ae.kind === "conflict"
                ? `the server refused the command: ${ae.message}`
                : ae.status === 503
                  ? "the control plane is at capacity — the queue is full; try again in a moment"
                  : ae.message;
        setSubmitError(message);
        toasts.push("err", message);
      } finally {
        setSubmitting(false);
      }
    },
    [refresh, toasts],
  );

  const cancel = useCallback(
    async (job: ControlJob) => {
      setBusyId(job.id);
      try {
        await apiPost<ControlJob>(`/control/jobs/${job.id}/cancel`, {});
        toasts.push("ok", `${job.id}: cancel accepted`);
        refresh();
      } catch (e) {
        const ae = e instanceof ApiError ? e : null;
        // 409 means the job finished before the cancel landed — that is the
        // server being right, not the console being wrong
        toasts.push(
          ae?.kind === "conflict" ? "info" : "err",
          ae?.kind === "conflict" ? `${job.id} already finished — nothing to cancel` : `${job.id}: ${ae?.message ?? String(e)}`,
        );
        refresh();
      } finally {
        setBusyId(null);
      }
    },
    [refresh, toasts],
  );

  const rows = jobs.data?.items ?? [];
  const caps = jobs.data?.caps;
  const inFlight = jobs.data?.in_flight ?? rows.filter((r) => !isTerminal(r.state)).length;

  return (
    <Page>
      <PageHeader
        title="Control Jobs"
        sub={
          <>
            Submit one bounded command at a time. The server validates it, runs it under a deadline, and
            reports the result — this page only shows what the server said.
          </>
        }
        actions={
          <Button variant="ghost" onClick={refresh} disabled={jobs.state === "refreshing"}>
            Refresh
          </Button>
        }
      />

      <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit,minmax(180px,1fr))" }}>
        <MetricCard label="Jobs in flight" value={fmtNum(inFlight)} meta={caps ? `ceiling ${caps.max_queue_depth}` : "queued + running"} />
        <MetricCard label="Concurrent slots" value={caps ? fmtNum(caps.max_concurrent_jobs) : "—"} meta="enforced by the server" />
        <MetricCard label="Retained jobs" value={fmtNum(jobs.data?.total ?? 0)} meta="newest first" />
        <MetricCard label="Last update" value={jobs.updatedAt ? fmtDateTime(Math.floor(jobs.updatedAt / 1000)) : "—"} meta={jobs.state} />
      </div>

      {jobs.error && (
        <ErrorState error={jobs.error} onRetry={refresh} />
      )}

      <Card title="Filters">
        <div style={{ display: "flex", gap: 12, flexWrap: "wrap", alignItems: "flex-end" }}>
          <Field label="State">
            <SelectInput
              aria-label="State"
              value={stateFilter}
              onChange={(e) => {
                setStateFilter(e.target.value);
                setOffset(0);
              }}
            >
              {STATE_FILTERS.map((s) => (
                <option key={s || "any"} value={s}>
                  {s || "any"}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="Operation">
            <SelectInput
              aria-label="Operation"
              value={opFilter}
              onChange={(e) => {
                setOpFilter(e.target.value);
                setOffset(0);
              }}
            >
              {OP_FILTERS.map((o) => (
                <option key={o || "any"} value={o}>
                  {o || "any"}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="Page size">
            <SelectInput
              aria-label="Page size"
              value={String(limit)}
              onChange={(e) => {
                setLimit(Number(e.target.value));
                setOffset(0);
              }}
            >
              {[10, 25, 50, 100].map((n) => (
                <option key={n} value={n}>
                  {n}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Button variant="ghost" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - limit))}>
            Previous
          </Button>
          <Button variant="ghost" disabled={rows.length < limit} onClick={() => setOffset(offset + limit)}>
            Next
          </Button>
        </div>
      </Card>

      <CommandComposer
        draft={draft}
        onChange={setDraft}
        targetIds={targetIds}
        onSubmit={submit}
        submitting={submitting}
        error={submitError}
        idempotencyKey={issues.length === 0 ? commandIdempotencyKey(draft.op, buildCommand(draft)) : undefined}
      />

      <Card
        title={`Jobs (${fmtNum(jobs.data?.total ?? rows.length)})`}
        actions={<JobStateLegend />}
      >
        {rows.length === 0 && !jobs.error ? (
          <EmptyState
            title="No control-plane jobs yet"
            body={
              jobs.data
                ? "Submit a command above — the server will validate it, run it under a deadline, and list it here."
                : "Loading the job list…"
            }
          />
        ) : (
          <div style={{ display: "grid", gap: 10 }}>
            {rows.map((j) => (
              <div
                key={j.id}
                data-testid="job-row"
                style={{ borderTop: "1px solid var(--line)", paddingTop: 8 }}
              >
                <div style={{ display: "flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
                  <code className="t-code">{j.id}</code>
                  <JobStateBadge state={j.state} cancelRequested={j.cancel_requested} />
                  <span className="chip">{j.operation}</span>
                  <span className="t-caption">{j.resource}</span>
                  <span style={{ marginLeft: "auto", display: "flex", gap: 6 }}>
                    <Button variant="ghost" className="sm" onClick={() => navigate(`/control/job?job=${encodeURIComponent(j.id)}`)}>
                      Open
                    </Button>
                    <Button
                      variant="ghost"
                      className="sm"
                      disabled={!canCancel(j.state) || busyId === j.id}
                      onClick={() => cancel(j)}
                      title={canCancel(j.state) ? "ask the server to cancel this job" : `${j.state} is terminal`}
                    >
                      Cancel
                    </Button>
                  </span>
                </div>
                <div className="t-caption" style={{ marginTop: 2 }}>
                  created {fmtDateTime(j.created_ts)} · deadline {fmtDateTime(j.deadline_ts)} ·{" "}
                  {j.operation === "crawl"
                    ? `${fmtNum(j.counters.pages_fetched)} page(s)`
                    : j.operation === "export"
                      ? `${fmtNum(j.counters.records_written)} record(s)`
                      : `${fmtNum(j.counters.records_extracted)} field(s)`}
                  {j.counters.failures > 0 && ` · ${fmtNum(j.counters.failures)} failure(s)`} ·{" "}
                  {j.artifacts.length} artifact(s)
                </div>
                {j.error && (
                  <div className="t-caption" style={{ marginTop: 2 }}>
                    <StatusChip status="failed" /> {j.error}
                  </div>
                )}
                {j.artifacts.length > 0 && (
                  <ArtifactList
                    artifacts={j.artifacts}
                    onDownload={(a) => navigate(`/artifacts?artifact=${encodeURIComponent(a.id)}`)}
                  />
                )}
              </div>
            ))}
          </div>
        )}
      </Card>

      <Card title="How this page stays honest">
        <ul className="t-caption" style={{ margin: 0, paddingLeft: 18, display: "grid", gap: 4 }}>
          <li>
            States come from the server's own job model ({["queued", "running", "completed", "failed", "cancelled"].join(", ")});
            illegal transitions are impossible because the server refuses them.
          </li>
          <li>
            Every submission carries an <code className="t-code">Idempotency-Key</code>, so a retry after a dropped
            connection returns the original job instead of starting a second one.
          </li>
          <li>
            Ceilings shown in the composer ({fmtNum(CEILINGS.maxPages)} pages, {fmtNum(CEILINGS.timeoutSecs)} s) are the
            server's constants; the server re-validates them regardless.
          </li>
          <li>No progress bar: a running job is running, and the counters update when the server reports them.</li>
        </ul>
      </Card>
    </Page>
  );
}
