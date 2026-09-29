// Jobs — split from Jobs.tsx; barrel re-exports keep every public path & name

import { useMemo, useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtMs, fmtNum } from "../../../lib/format";
import type { EventsResponse, FailuresResponse, JobsResponse, RetriesResponse } from "../../../lib/types";
import { Card, PageHeader, StatusChip, MetricCard, EmptyState } from "../../../components/ui";
import { DataTable, type Column } from "../../../components/Table";
import { Drawer } from "../../../components/Overlays";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";
import { Timeline } from "../../../components/patterns";

export default function Jobs() {
  const jobs = usePolled((s) => apiGet<JobsResponse>("/jobs", { signal: s }), 5000);
  const events = usePolled((s) => apiGet<EventsResponse>("/events?limit=200", { signal: s }), 5000);
  const failures = usePolled((s) => apiGet<FailuresResponse>("/failures?limit=50", { signal: s }), 8000);
  const retries = usePolled((s) => apiGet<RetriesResponse>("/retries", { signal: s }), 8000);
  const [open, setOpen] = useState<{ target: string; idx: number } | null>(null);

  const rows = useMemo(() => jobs.data?.job.jobs ?? [], [jobs.data]);
  const targets = useMemo(() => rows.map((r) => r.target), [rows]);

  const targetEvents = useMemo(() => {
    if (!open || !events.data) return [];
    return events.data.events.filter((e) => e.detail["target"] === open.idx || targets[Number(e.detail["target"] ?? -1)] === open.target);
  }, [open, events.data, targets]);

  const cols: Column<JobsResponse["job"]["jobs"][number]>[] = [
    { key: "target", label: "Target", render: (r) => <code className="t-code">{r.target}</code> },
    { key: "written", label: "Written", numeric: true, render: (r) => fmtNum(r.written) },
    { key: "revalidated", label: "Revalidated", numeric: true, hideable: true, render: (r) => fmtNum(r.revalidated) },
    { key: "blocked", label: "Blocked", numeric: true, render: (r) => <span style={r.blocked ? { color: "var(--warn)" } : undefined}>{fmtNum(r.blocked)}</span> },
    { key: "failed", label: "Failed", numeric: true, render: (r) => <span style={r.failed ? { color: "var(--err)" } : undefined}>{fmtNum(r.failed)}</span> },
    { key: "quarantined", label: "Quarantined", numeric: true, render: (r) => fmtNum(r.quarantined) },
  ];

  return (
    <Page>
      <PageHeader
        title="Jobs"
        sub={
          jobs.data
            ? <>run <code className="t-code">{jobs.data.job.run}</code> · {jobs.data.job.status} · {fmtMs(jobs.data.job.uptime_ms)}{" "}
                {jobs.data.note ? <span className="t-caption">· {jobs.data.note}</span> : null}</>
            : "no run data"
        }
        actions={jobs.data && <StatusChip status={jobs.data.job.status} />}
      />

      <div className="grid grid-metrics" role="list" aria-label="Run rollup">
        <MetricCard label="Run status" value={jobs.data?.job.status ?? "—"} tone={jobs.data?.job.status === "running" ? "info" : "idle"} meta={<>uptime {fmtMs(jobs.data?.job.uptime_ms)}</>} />
        <MetricCard label="Recent failures" value={fmtNum(failures.data?.total)} tone={(failures.data?.total ?? 0) > 0 ? "err" : "idle"} meta={<>{failures.data?.state === "unavailable" ? "no run attached" : "live accounting"}</>} />
        <MetricCard label="Retries" value={fmtNum(retries.data?.accounting_retries)} tone={(retries.data?.accounting_retries ?? 0) > 2 ? "warn" : "idle"} meta={<>delays: {retries.data?.recent_delays_ms.length ?? 0} recent</>} />
        <MetricCard label="Targets in run" value={fmtNum(rows.length)} meta={<span className="t-caption">per-target tally, not queue jobs</span>} />
      </div>

      <DataTable
        caption="job tallies per target"
        columns={cols}
        rows={rows}
        rowKey={(r) => r.target}
        loading={jobs.state === "connecting"}
        error={jobs.error}
        onRowClick={(r) => setOpen({ target: r.target, idx: targets.indexOf(r.target) })}
        emptyTitle="No run has produced tallies"
        emptyBody="Jobs appear while the engine runs and fall back to the last persisted run afterwards."
      />

      {open && (
        <Drawer
          title={<span>Target <code className="t-code">{open.target}</code></span>}
          onClose={() => setOpen(null)}
        >
          <Card title="Lifecycle (live event ring)">
            {targetEvents.length ? (
              <Timeline
                label="target job lifecycle"
                items={targetEvents.slice(0, 24).map((e) => ({
                  title: e.kind.replaceAll("_", " "),
                  tone: e.kind.includes("fail") ? "err" : e.kind.includes("completed") ? "ok" : e.kind.includes("retry") ? "warn" : "" ,
                  meta: (
                    <>
                      {new Date(e.ts * 1000).toLocaleTimeString("en-GB")} · job #{String(e.job ?? "—")}
                      {e.detail["url"] ? <> · <code className="t-code">{String(e.detail["url"])}</code></> : null}
                    </>
                  ),
                }))}
              />
            ) : (
              <EmptyState icon={<Icon.Queue size={18} />} title="No events for this target" body="The event ring covers the live run only; between runs it is empty." />
            )}
          </Card>
          <Card title="Retry & failure detail">
            <dl className="kv">
              <dt>recent retry delays</dt>
              <dd>{retries.data?.recent_delays_ms.length ? retries.data.recent_delays_ms.map((d) => `${d}ms`).join(" · ") : "—"}</dd>
              <dt>recent failures</dt>
              <dd>{failures.data?.items.length ? `${failures.data.items.length} entr${failures.data.items.length === 1 ? "y" : "ies"} (see Activity)` : "—"}</dd>
            </dl>
            {failures.data?.items.length ? (
              <ul style={{ margin: "8px 0 0", paddingLeft: 18, fontSize: 12.5, color: "var(--text-2)" }}>
                {failures.data.items.slice(0, 8).map((f, i) => (
                  <li key={i}><code className="t-code">{String(f.url ?? "?")}</code> — {String(f.error ?? f.status ?? "failed")}</li>
                ))}
              </ul>
            ) : null}
          </Card>
          <p className="t-caption">
            Worker assignment is per-request inside a target job (the supervisor pools them):
            see the queue page for pool state. Distributed leases live in Redis — exposed via
            <code className="t-code"> harvest queue </code> on the CLI.
          </p>
        </Drawer>
      )}
    </Page>
  );
}
