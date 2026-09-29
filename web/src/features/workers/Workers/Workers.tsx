// Workers — split from Workers.tsx; barrel re-exports keep every public path & name

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtNum, fmtPct } from "../../../lib/format";
import type { WorkerHealth } from "../../../lib/types";
import { Card, MetricCard, PageHeader, Skeleton, StatusChip, EmptyState } from "../../../components/ui";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";

export default function Workers() {
  const w = usePolled((s) => apiGet<WorkerHealth>("/workers", { signal: s }), 5000);
  const snap = w.data?.snapshot ?? null;

  return (
    <Page>
      <PageHeader
        title="Workers"
        sub={
          w.data
            ? <>component state: <strong>{w.data.state}</strong>{snap?.draining ? " · draining" : ""}</>
            : "supervisor pool for the live process"
        }
        actions={<StatusChip status={w.data?.state ?? "unavailable"} />}
      />

      {w.state === "connecting" && <Card><Skeleton h={80} /></Card>}

      {w.data?.state === "unavailable" && (
        <Card>
          <EmptyState
            icon={<Icon.Workers size={18} />}
            title="No workers registered"
            body={
              <>
                Workers exist only while a run is active — the supervisor spawns the pool
                at run start and drains it at completion. Distributed <code className="t-code">harvest worker</code>{" "}
                shards keep their state in Redis; use <code className="t-code">harvest queue</code> for that view.
              </>
            }
          />
        </Card>
      )}

      {snap && (
        <>
          <div className="grid grid-metrics" role="list" aria-label="Pool rollup">
            <MetricCard label="Registered" value={fmtNum(snap.registered)} tone="ok" meta={<>{snap.deaths} deaths · {snap.total_restarts} restarts</>} />
            <MetricCard label="Busy" value={fmtNum(snap.busy)} tone="info" meta={<>utilization {fmtPct(snap.utilization)}</>} />
            <MetricCard label="Utilization" value={fmtPct(snap.utilization)} tone={snap.utilization > 0.9 ? "warn" : "ok"} />
            <MetricCard label="State" value={snap.draining ? "draining" : "running"} tone={snap.draining ? "warn" : "ok"} />
          </div>
          <div className="grid grid-3" role="list" aria-label="Workers">
            {snap.workers.map((wk) => {
              const fresh = wk.idle_secs < 10 ? "fresh" : wk.idle_secs < 60 ? "stale" : "dead";
              return (
                <article key={wk.id} className="card metric card--hover" aria-label={`worker ${wk.id}`}>
                  <div className="metric-label t-label" style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <i className="dot ok live" aria-hidden="true" />
                    worker #{wk.id}
                    <span className="chip" style={{ marginLeft: "auto" }}>{wk.state}</span>
                  </div>
                  <div className="metric-value t-metric tnum">{fmtNum(wk.jobs_done)}<small>jobs done</small></div>
                  <div className="metric-meta" style={{ display: "flex", alignItems: "center", gap: 10 }}>
                    <div style={{ display: "grid", gap: 2 }}>
                      <span className="t-caption">idle {fmtNum(wk.idle_secs)}s</span>
                      <span className={`hb ${fresh === "fresh" ? "fresh" : fresh}`} role="img" aria-label={`heartbeat ${fresh}`}>
                        <i style={{ width: fresh === "fresh" ? "100%" : fresh === "stale" ? "45%" : "8%" }} />
                      </span>
                    </div>
                    <span className="t-caption" style={{ marginLeft: "auto" }}>
                      {fmtNum(wk.jobs_failed)} failed · {fmtNum(wk.restarts)} restarts
                    </span>
                  </div>
                </article>
              );
            })}
          </div>
        </>
      )}
    </Page>
  );
}
