/**
 * Command Center — one-glance operations view over the live engine.
 *
 * Composes four REAL endpoints only (each already documented in
 * `docs/web-client.md` and pinned by the API contract tests):
 *   GET /runtime   live snapshot (or {error:"no run active"} between runs)
 *   GET /storage   per-lane store observatory
 *   GET /alerts    alert engine state + live items
 *   GET /events    newest structured events (capped tail of the ring)
 *
 * Between runs /runtime answers `no run active` — that is shown as an
 * honest idle card, never faked into zeroes.
 */
import { apiGet } from "../../lib/api";
import { usePolled } from "../../hooks/usePolled";
import { isRuntimeSnapshot } from "../../lib/types";
import type { AlertsResponse, EventsResponse, RuntimeResponse, RunsResponse, StorageLane, StorageResponse } from "../../lib/types";
import { fmtNum, fmtBytes, fmtRate, humanDuration, statusLabel } from "../../lib/dataFormatters";
import { Page } from "../../components/AppShell";
import { Card, EmptyState, MetricCard, PageHeader, StatusChip } from "../../components/ui";
import { ActivityTimeline } from "../../components/patterns/ActivityTimeline";
import { useConnection } from "../../lib/connection";

const LANE_ORDER = ["output", "history", "events", "audit", "checkpoint", "revalidation", "quarantine"];

export default function CommandCenter() {
  const conn = useConnection();
  const runtime = usePolled((s) => apiGet<RuntimeResponse>("/runtime", { signal: s }), 5000);
  const storage = usePolled((s) => apiGet<StorageResponse>("/storage", { signal: s }), 15000);
  const alerts = usePolled((s) => apiGet<AlertsResponse>("/alerts", { signal: s }), 15000);
  const events = usePolled((s) => apiGet<EventsResponse>("/events?limit=64", { signal: s }), 5000);
  const runs = usePolled((s) => apiGet<RunsResponse>("/runs?limit=1", { signal: s }), 15000);

  const snap = runtime.data && isRuntimeSnapshot(runtime.data) ? runtime.data : null;
  const lanes = storage.data ?? {};
  const orderedLanes: Array<readonly [string, StorageLane]> = [];
  for (const k of LANE_ORDER) {
    const l = lanes[k];
    if (l) orderedLanes.push([k, l]);
  }
  for (const [k, l] of Object.entries(lanes)) {
    if (!LANE_ORDER.includes(k) && l) orderedLanes.push([k, l]);
  }
  const degradedLanes = orderedLanes.filter(([, l]) => l.configured && !l.exists);

  return (
    <Page>
      <PageHeader
        title="Command Center"
        sub={
          snap
            ? `run “${snap.engine.run}” · ${statusLabel(snap.engine.status)} · uptime ${humanDuration(snap.engine.uptime_ms)}`
            : runtime.error
              ? "runtime snapshot unreachable"
              : "idle — no run in progress"
        }
      />

      <div className="grid grid-metrics">
        <MetricCard
          label="Engine"
          value={snap ? <StatusChip status={snap.engine.status} /> : <StatusChip status="idle" />}
          meta={conn.health ? (conn.health.running ? "health: running" : "health: not running") : "health: unknown"}
        />
        <MetricCard
          label="Queue"
          value={snap ? fmtNum(snap.queue.depth) : "—"}
          unit={snap && snap.queue.max_depth > 0 ? `/ ${fmtNum(snap.queue.max_depth)}` : undefined}
          meta={snap ? `${fmtNum(snap.queue.inflight)} inflight · ${fmtRate(snap.accounting.jobs_per_sec)} jobs/s` : "no live run"}
          tone={snap && snap.queue.rejected > 0 ? "warn" : undefined}
        />
        <MetricCard
          label="Written"
          value={snap ? fmtNum(snap.metrics.records_written) : runs.data ? fmtNum(runs.data.page.total) : "—"}
          unit={snap ? undefined : "runs in ledger"}
          meta={snap ? `${fmtNum(snap.metrics.pages_fetched)} pages · ${fmtNum(snap.metrics.records_quarantined)} quarantined` : "history totals instead of live counters"}
        />
        <MetricCard
          label="Alerts"
          value={alerts.data ? fmtNum(alerts.data.items.length) : "—"}
          meta={alerts.data ? `${fmtNum(alerts.data.fired)} fired · ${fmtNum(alerts.data.notified)} notified` : undefined}
          tone={alerts.data && alerts.data.items.length > 0 ? "err" : "ok"}
        />
      </div>

      <div className="grid grid-2">
        <Card title="Storage lanes" hover>
          {orderedLanes.length === 0 ? (
            <EmptyState title="No storage observatory yet" body="GET /storage answered without lanes." />
          ) : (
            <table className="data">
              <thead>
                <tr>
                  <th>lane</th>
                  <th>state</th>
                  <th>kind</th>
                  <th>path</th>
                  <th>bytes</th>
                </tr>
              </thead>
              <tbody>
                {orderedLanes.map(([name, l]) => (
                  <tr key={name}>
                    <td>{name}</td>
                    <td>
                      <StatusChip
                        status={!l.configured ? "disabled" : l.exists ? "healthy" : "unavailable"}
                        label={statusLabel(!l.configured ? "disabled" : l.exists ? "healthy" : "unavailable")}
                      />
                    </td>
                    <td className="t-muted">{l.kind || "—"}</td>
                    <td className="t-muted">{l.path || "—"}</td>
                    <td>{l.bytes === null ? "—" : fmtBytes(l.bytes)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          {degradedLanes.length > 0 && (
            <p className="t-caption t-err">
              configured lanes missing on disk: {degradedLanes.map(([k]) => k).join(", ")} —
              a run that should have written them did not.
            </p>
          )}
        </Card>

        <Card title="Recent activity" hover>
          <ActivityTimeline events={events.data?.events ?? []} cap={60} dropped={events.data?.dropped} />
          {(!events.data || events.data.events.length === 0) && !events.error && (
            <p className="t-caption t-muted">The event ring is empty — activity appears here while a run executes.</p>
          )}
        </Card>
      </div>
    </Page>
  );
}
