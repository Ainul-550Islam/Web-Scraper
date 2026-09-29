// Overview — split from Overview.tsx; barrel re-exports keep every public path & name

import { useMemo } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { useConnection } from "../../../lib/connection";
import { fmtAgo, fmtMs, fmtNum, fmtPct, fmtRate } from "../../../lib/format";
import type { DiagnosticsResponse, EventsResponse, RuntimeResponse, RuntimeSnapshot, StatsResponse } from "../../../lib/types";
import { Card, ErrorState, HealthCell, MetricCard, PageHeader, StatusChip } from "../../../components/ui";
import { yn } from "./yn";
import { pretty } from "./pretty";
import { brief } from "./brief";
import { buildIntelligence } from "./buildIntelligence";
import { HealthSummary } from "./HealthSummary";

export default function Overview() {
  const conn = useConnection();
  const stats = usePolled((s) => apiGet<StatsResponse>("/stats", { signal: s }), 5000);
  const rt = usePolled((s) => apiGet<RuntimeResponse>("/runtime", { signal: s }), 5000);
  const diag = usePolled((s) => apiGet<DiagnosticsResponse>("/diagnostics", { signal: s }), 10000);
  const events = usePolled((s) => apiGet<EventsResponse>("/events?limit=120", { signal: s }), 8000);

  const snapshot: RuntimeSnapshot | null =
    rt.data && !("error" in rt.data) ? rt.data : null;
  // /stats is the durable counter set; the runtime snapshot shares it while live
  const m = stats.data?.metrics ?? snapshot?.metrics ?? null;
  const proxies = stats.data?.proxies ?? [];

  const writeSamples = useMemo(() => {
    if (!events.data) return [];
    let acc = 0;
    return events.data.events
      .filter((e) => e.kind === "sink_write_completed")
      .map((e) => (acc += Number(e.detail["records"] ?? 0)))
      .reverse();
  }, [events.data]);

  const mSuccess = m && m.requests_total > 0 ? m.requests_ok / m.requests_total : null;
  const engineRunning = conn.health?.running ?? false;

  const intelligence = useMemo(
    () => buildIntelligence(conn.config, snapshot, m, conn.alerts, mSuccess, engineRunning),
    [conn.config, snapshot, m, conn.alerts, mSuccess, engineRunning],
  );

  const avgStoreMs = m && m.storage_writes > 0 ? m.storage_write_ms_total / m.storage_writes : null;

  return (
    <>
      <PageHeader
        title="Overview"
        sub={
          conn.config
            ? `engine “${conn.config.name}” · updated ${fmtAgo(Math.floor((conn.updatedAt ?? 0) / 1000))}`
            : conn.health
              ? "connected"
              : "awaiting admin API"
        }
        actions={
          <StatusChip status={engineRunning ? "running" : conn.health ? "idle" : "unavailable"} />
        }
      />

      {conn.state === "offline" && !conn.health ? (
        <Card>
          <ErrorState error={conn.error} onRetry={conn.refreshAll} />
        </Card>
      ) : (
        <div className="grid grid-metrics" role="list" aria-label="Key metrics">
          <MetricCard
            label="Engine"
            value={engineRunning ? "running" : conn.health ? "idle" : "offline"}
            tone={engineRunning ? "ok" : conn.health ? "info" : "err"}
            meta={snapshot ? `uptime ${fmtMs(snapshot.engine.uptime_ms)}` : "no run attached"}
            live={engineRunning}
          />
          <MetricCard
            label="Records written"
            value={fmtNum(m?.records_written)}
            tone="ok"
            meta={<span className="t-caption">process lifetime counter</span>}
            spark={writeSamples.length > 1 ? writeSamples : undefined}
          />
          <MetricCard
            label="Pages fetched"
            value={fmtNum(m?.pages_fetched)}
            meta={<>throughput {fmtRate(snapshot?.accounting.jobs_per_sec ?? 0)} jobs/s</>}
          />
          <MetricCard
            label="Queue depth"
            value={fmtNum(snapshot?.queue.depth)}
            tone={(snapshot?.queue.depth ?? 0) > 50 ? "warn" : "idle"}
            meta={<>inflight {fmtNum(snapshot?.queue.inflight)} · rejected {fmtNum(snapshot?.queue.rejected)}</>}
          />
          <MetricCard
            label="Workers"
            value={fmtNum(snapshot?.workers.registered)}
            tone={snapshot && snapshot.workers.deaths > 0 ? "warn" : "ok"}
            meta={<>busy {fmtNum(snapshot?.workers.busy)} · util {snapshot ? fmtPct(snapshot.workers.utilization) : "—"}</>}
          />
          <MetricCard
            label="Proxy egress"
            value={fmtNum(proxies.length)}
            tone="ok"
            meta={
              proxies[0]
                ? <>ewma {fmtMs(proxies[0].latency_ewma_ms)} · {fmtPct(proxies[0].success_ewma)} ok</>
                : <span className="t-caption">direct egress</span>
            }
          />
          <MetricCard
            label="Request success"
            value={mSuccess === null ? "—" : fmtPct(mSuccess)}
            tone={mSuccess !== null && mSuccess < 0.98 ? "warn" : "ok"}
            meta={<>retries {fmtNum(m?.requests_retry)} · blocked {fmtNum(m?.requests_blocked)}</>}
          />
          <MetricCard
            label="Quarantined"
            value={fmtNum(m?.records_quarantined)}
            tone={(m?.records_quarantined ?? 0) > 0 ? "warn" : "idle"}
            meta={<>schema drifts {fmtNum(m?.schema_drifts)}</>}
          />
          <MetricCard
            label="Failed jobs"
            value={fmtNum(m?.jobs_failed)}
            tone={(m?.jobs_failed ?? 0) > 0 ? "err" : "idle"}
            meta={<>pipeline {fmtNum(m?.pipeline_failures)} · abandoned {fmtNum(m?.domains_abandoned)}</>}
          />
          <MetricCard
            label="Checkpoints"
            value={fmtNum(m?.checkpoints_written)}
            meta={<>{fmtNum(m?.storage_writes)} storage writes · {fmtMs(avgStoreMs)} avg</>}
          />
          <MetricCard
            label="Storage bytes"
            value={fmtNum(snapshot?.accounting.bytes_received)}
            meta={<span className="t-caption">received on fetch lane (not disk size)</span>}
          />
          <MetricCard
            label="Alerts fired"
            value={fmtNum(m?.alerts_fired)}
            tone={(conn.alerts?.items.length ?? 0) > 0 ? "warn" : "idle"}
            meta={<>{fmtNum(conn.alerts?.notified)} notified</>}
          />
        </div>
      )}

      <HealthSummary stats={stats.data} rt={snapshot} />

      <div className="grid grid-2">
        <Card
          title="System Intelligence"
          actions={<span className="chip idle">derived from live counters — no inference</span>}
        >
          <div style={{ display: "grid", gap: "var(--s-4)" }}>
            {intelligence.map((row) => (
              <section key={row.label} aria-label={row.label} style={{ display: "grid", gap: 3 }}>
                <div className="t-label">
                  <i className={`dot ${row.tone}`} style={{ marginRight: 7 }} aria-hidden="true" />
                  {row.label}
                </div>
                {row.lines.map((l, i) => (
                  <p key={i} style={{ margin: 0, fontSize: 13, color: "var(--text-2)", lineHeight: 1.55 }}>
                    {l}
                  </p>
                ))}
              </section>
            ))}
          </div>
        </Card>

        <Card
          title="System health"
          actions={
            diag.data && (
              <StatusChip
                status={diag.data.overall === "unhealthy" ? "error" : diag.data.overall}
                label={diag.data.overall}
              />
            )
          }
        >
          <div className="health-grid">
            <HealthCell
              name="Engine"
              status={engineRunning ? "healthy" : conn.health ? "degraded" : "unavailable"}
              detail={engineRunning ? "run active" : "idle between runs"}
            />
            <HealthCell
              name="API"
              status={conn.state === "offline" ? "unavailable" : "healthy"}
              detail={conn.latencyMs !== null ? `${conn.latencyMs} ms rtt` : undefined}
            />
            <HealthCell
              name="Scheduler"
              status={conn.config ? (conn.config.schedule.every_secs > 0 ? "healthy" : "disabled") : "unavailable"}
              detail={
                conn.config
                  ? conn.config.schedule.every_secs > 0
                    ? `every ${conn.config.schedule.every_secs}s`
                    : "once-mode"
                  : undefined
              }
            />
            {diag.data?.components
              .filter((c) => !["runtime", "config"].includes(c.name))
              .map((c) => (
                <HealthCell
                  key={c.name}
                  name={pretty(c.name)}
                  status={c.status === "unhealthy" ? "degraded" : c.status}
                  detail={brief(c.detail)}
                />
              ))}
            {conn.config && (
              <HealthCell
                name="Redis"
                status={conn.config.redis_url ? "degraded" : "disabled"}
                detail={
                  conn.config._backends?.redis === false
                    ? "binary built without redis"
                    : conn.config.redis_url
                      ? "URL configured (value never shown)"
                      : "not configured"
                }
              />
            )}
            {conn.config && (
              <HealthCell
                name="SQL backends"
                status={
                  conn.config.output.kind === "sqlite" || conn.config.output.kind === "postgres"
                    ? "healthy"
                    : "disabled"
                }
                detail={`built: sqlite:${yn(conn.config._backends?.sqlite)} pg:${yn(conn.config._backends?.postgres)} redis:${yn(conn.config._backends?.redis)}`}
              />
            )}
            <HealthCell
              name="Pipeline"
              status={
                m && m.pipeline_failures + m.schema_drifts > 0
                  ? "degraded"
                  : stats.data
                    ? "healthy"
                    : "unavailable"
              }
              detail={m ? `${fmtNum(m.pipeline_failures)} failures · ${fmtNum(m.schema_drifts)} drifts` : undefined}
            />
          </div>
        </Card>
      </div>
    </>
  );
}
