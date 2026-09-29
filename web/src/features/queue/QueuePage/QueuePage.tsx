// QueuePage — split from QueuePage.tsx; barrel re-exports keep every public path & name

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtMs, fmtNum } from "../../../lib/format";
import type { QueueHealth } from "../../../lib/types";
import { Card, MetricCard, MiniBars, PageHeader, Skeleton, StatusChip, EmptyState } from "../../../components/ui";
import { ChartCard } from "../../../components/patterns";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";

export default function QueuePage() {
  const q = usePolled((s) => apiGet<QueueHealth>("/queue", { signal: s }), 4000);
  const st = q.data?.stats ?? null;

  return (
    <Page>
      <PageHeader
        title="Queue"
        sub={q.data ? <>component state: <strong>{q.data.state}</strong> · mode {st?.mode ?? "fifo"} (in-process scheduler)</> : "in-process scheduler queue"}
        actions={<StatusChip status={q.data?.state ?? "unavailable"} />}
      />

      {q.state === "connecting" && <Card><Skeleton h={80} /></Card>}

      {q.data?.state !== "healthy" && !st && (
        <Card>
          <EmptyState
            icon={<Icon.Queue size={18} />}
            title={q.data?.state === "degraded" ? "Queue idle" : "No live queue"}
            body="Queue state belongs to the active run; between runs the in-process queue is empty by design. Distributed depth lives in Redis (CLI: harvest queue)."
          />
        </Card>
      )}

      {st && (
        <>
          <div className="grid grid-metrics" role="list" aria-label="Queue metrics">
            <MetricCard label="Depth (waiting)" value={fmtNum(st.depth)} tone={st.depth > 50 ? "warn" : "idle"} meta={<>{st.max_depth === 0 ? "unbounded cap" : `cap ${fmtNum(st.max_depth)}`}</>} />
            <MetricCard label="Inflight" value={fmtNum(st.inflight)} tone="info" meta={<>per-target concurrency × workers</>} />
            <MetricCard label="Popped / pushed" value={`${fmtNum(st.popped)} / ${fmtNum(st.pushed)}`} meta={<span className="t-caption">cumulative this run</span>} />
            <MetricCard label="Rejected" value={fmtNum(st.rejected)} tone={st.rejected ? "err" : "idle"} meta={<span className="t-caption">depth-cap refusals</span>} />
            <MetricCard label="Expired" value={fmtNum(st.expired)} tone={st.expired ? "warn" : "idle"} meta={<span className="t-caption">older than job_max_age_secs</span>} />
            <MetricCard label="Avg wait" value={fmtMs(q.data?.avg_queue_wait_ms || null)} meta={<span className="t-caption">push → pop latency</span>} />
          </div>

          <ChartCard title="Per-target pressure" meta={<span className="t-caption">live gauges from GET /queue — no history, so no lines</span>}>
            {st.per_target_depths.length + st.per_target_active.length === 0 ? (
              <p className="t-caption">No per-target samples yet this run.</p>
            ) : (
              <div className="grid grid-2">
                <div>
                  <div className="t-label" style={{ marginBottom: 6 }}>Waiting by target index</div>
                  <MiniBars values={st.per_target_depths.map(([, v]) => v)} label="waiting per target" />
                </div>
                <div>
                  <div className="t-label" style={{ marginBottom: 6 }}>Active by target index</div>
                  <MiniBars values={st.per_target_active.map(([, v]) => v)} label="active per target" />
                  <p className="t-caption" style={{ marginTop: 6 }}>
                    Indices follow the config target order — see the pipeline page for id mapping.
                  </p>
                </div>
              </div>
            )}
          </ChartCard>
        </>
      )}
    </Page>
  );
}
