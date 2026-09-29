// Proxies — split from Proxies.tsx; barrel re-exports keep every public path & name

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtMs, fmtNum, fmtPct } from "../../../lib/format";
import type { StatsResponse } from "../../../lib/types";
import { Card, MetricCard, PageHeader, Segmented, StatusChip } from "../../../components/ui";
import { DataTable, type Column } from "../../../components/Table";
import { FilterBar, SearchInput, Tooltip } from "../../../components/patterns";
import { Page } from "../../../components/AppShell";
import { useConnection } from "../../../lib/connection";
import { Icon } from "../../../components/Icons";
import { useMemo, useState } from "react";
import type { ProxyPoolEntry } from "./ProxyPoolEntry";
import type { ProxyPoolResponse } from "./ProxyPoolResponse";


/**
 * Egress observability from the engine's dedicated `GET /proxies` snapshot
 * (state machine + live EWMA), cross-referenced with the declared pool in
 * the redacted `GET /config`. Proxy URLs embed credentials, so they are
 * NEVER read out or rendered — ids, regions and health only (§15).
 */
export default function Proxies() {
  const pool = usePolled((s) => apiGet<ProxyPoolResponse>("/proxies", { signal: s }), 6000);
  const stats = usePolled((s) => apiGet<StatsResponse>("/stats", { signal: s }), 6000);
  const conn = useConnection();
  const [q, setQ] = useState("");
  const [only, setOnly] = useState<"all" | "healthy" | "degraded" | "down" | "unprobed">("all");

  const rows = useMemo(() => {
    const src = pool.data?.entries ?? stats.data?.proxies?.map((p) => ({ ...p, state: "unknown" })) ?? [];
    const needle = q.trim().toLowerCase();
    return (src as ProxyPoolEntry[]).filter(
      (r) =>
        (only === "all" || r.state === only) &&
        (!needle || r.id.toLowerCase().includes(needle) || (r.region ?? "").toLowerCase().includes(needle)),
    );
  }, [pool.data, stats.data, q, only]);

  const cols: Column<ProxyPoolEntry>[] = [
    {
      key: "id", label: "Proxy", sortValue: (r) => r.id, render: (r) => (
        <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
          <Icon.Proxies size={12} /> <code className="t-code">{r.id}</code>
          {r.id === "direct" && (
            <Tooltip tip="no proxy — direct egress from this host">
              <span className="chip idle">direct</span>
            </Tooltip>
          )}
        </span>
      ),
    },
    { key: "region", label: "Region", render: (r) => r.region ?? "—" },
    {
      key: "state", label: "State", sortValue: (r) => r.state, render: (r) => (
        <StatusChip
          status={r.state === "healthy" ? "ok" : r.state === "down" ? "error" : r.state === "unprobed" || r.state === "unknown" ? "idle" : "degraded"}
          label={r.state}
        />
      ),
    },
    {
      key: "health", label: "Success EWMA", numeric: true, hideable: true,
      sortValue: (r) => r.success_ewma,
      render: (r) => (
        <span style={{ color: r.success_ewma >= 0.99 ? "var(--ok)" : r.success_ewma >= 0.9 ? "var(--warn)" : "var(--err)" }}>
          {fmtPct(r.success_ewma)}
        </span>
      ),
    },
    { key: "latency_ewma_ms", label: "Latency", numeric: true, sortValue: (r) => r.latency_ewma_ms, render: (r) => fmtMs(r.latency_ewma_ms) },
    { key: "requests", label: "Requests", numeric: true, sortValue: (r) => r.requests, render: (r) => fmtNum(r.requests) },
    { key: "failures", label: "Failures", numeric: true, render: (r) => <span style={r.failures ? { color: "var(--err)" } : undefined}>{fmtNum(r.failures)}</span> },
    { key: "inflight", label: "Inflight", numeric: true, hideable: true, render: (r) => fmtNum(r.inflight) },
    {
      key: "quota", label: "Per-proxy cap", hideable: true, numeric: true, defaultHidden: true,
      render: () => (conn.config?.max_inflight_per_proxy ? fmtNum(conn.config.max_inflight_per_proxy) : "unlimited"),
    },
  ];

  const p = pool.data;
  const totalReq = rows.reduce((a, r) => a + r.requests, 0);
  const totalFail = rows.reduce((a, r) => a + r.failures, 0);
  const declared = conn.config?.proxies.length ?? 0;

  return (
    <Page>
      <PageHeader
        title="Proxy Pool"
        sub={
          <>
            {p ? `${fmtNum(p.total)} egress · ${fmtNum(p.healthy)} healthy · ${fmtNum(p.degraded)} degraded · ${fmtNum(p.down)} down · ${fmtNum(p.unprobed)} unprobed` : `${rows.length} egress`}
            {" · "}latency/quarantine are live EWMA values; refusals while an egress is saturated count as <code className="t-code">inflight_denied</code>
          </>
        }
      />
      <div className="grid grid-metrics" role="list" aria-label="Pool summary">
        <MetricCard label="Egress count" value={fmtNum(p?.total ?? rows.length)} meta={<>{declared} declared in config</>} />
        <MetricCard label="Pooled requests" value={fmtNum(totalReq)} tone="info" />
        <MetricCard label="Failures" value={fmtNum(totalFail)} tone={totalFail > 0 ? "warn" : "ok"} />
        <MetricCard
          label="Degraded / down"
          value={`${fmtNum(p?.degraded ?? 0)} / ${fmtNum(p?.down ?? 0)}`}
          tone={(p?.degraded ?? 0) + (p?.down ?? 0) > 0 ? "err" : "idle"}
          meta={<span className="t-caption">EWMA &lt; 90% is degraded; probes gate return</span>}
        />
        <MetricCard
          label="Inflight denied"
          value={fmtNum(p?.inflight_denied)}
          tone={(p?.inflight_denied ?? 0) > 0 ? "warn" : "idle"}
          meta={<span className="t-caption">cooldown pressure (cap {conn.config?.max_inflight_per_proxy ? fmtNum(conn.config.max_inflight_per_proxy) : "off"})</span>}
        />
      </div>

      <FilterBar>
        <SearchInput label="filter proxies" placeholder="id or region…" value={q} onChange={setQ} />
        <Segmented
          ariaLabel="state filter"
          value={only}
          onChange={setOnly}
          options={[
            { value: "all", label: "All" },
            { value: "healthy", label: "Healthy" },
            { value: "degraded", label: "Degraded" },
            { value: "down", label: "Down" },
            { value: "unprobed", label: "Unprobed" },
          ]}
        />
        <span className="spacer" />
        <span className="t-caption">{rows.length} shown{q || only !== "all" ? ` of ${p?.total ?? "?"}` : ""}</span>
      </FilterBar>

      <Card title="Egress table" hover>
        <DataTable
          caption="proxy pool snapshot (GET /proxies)"
          columns={cols}
          rows={rows}
          rowKey={(r) => r.id}
          loading={pool.state === "connecting" && stats.state === "connecting"}
          error={pool.error && stats.error ? pool.error : null}
          emptyTitle="No egress configured"
          emptyBody="Add entries under `proxies` in the config to see rotation, health-based failover, and per-egress counters here."
        />
      </Card>

      <p className="t-caption" style={{ marginTop: -8 }}>
        Proxy credentials never reach the browser: the server redacts them in
        <code className="t-code"> GET /config</code> and this page renders only
        ids, regions, and health. Usage is counted per egress; refusal
        backoff and circuit-breaking stay engine-side.
      </p>
    </Page>
  );
}
