// HealthSummary — six honest groups over the data Overview already polls
// (+ its own /storage poll, 10 s, same bounded cadence as the Storage page).
// Every number comes from a typed response; rows show "—" when the engine
// has not answered, never a placeholder zero.

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { useConnection } from "../../../lib/connection";
import type { RuntimeSnapshot, StatsResponse, StorageResponse } from "../../../lib/types";
import { Card, StatusChip } from "../../../components/ui";
import { fmtMs, fmtNum } from "../../../lib/format";

interface Row {
  label: string;
  value: string;
  tone?: "ok" | "warn" | "err" | "idle";
}

const DASH = "—";

function groupRows(rows: Row[]) {
  return (
    <div className="welcome-panel">
      {rows.map((r) => (
        <span key={r.label} className="wp-row" style={{ flexDirection: "row", justifyContent: "space-between", gap: 10 }}>
          <span className="t-label">{r.label}</span>
          {r.tone ? (
            <StatusChip status={r.tone} dot={false} label={r.value} />
          ) : (
            <span className="t-num tnum" style={{ textAlign: "right" }}>{r.value}</span>
          )}
        </span>
      ))}
    </div>
  );
}

export function HealthSummary({
  stats,
  rt,
}: {
  stats: StatsResponse | null;
  rt: RuntimeSnapshot | null;
}) {
  const conn = useConnection();
  const store = usePolled((s) => apiGet<StorageResponse>("/storage", { signal: s }), 10_000);
  const m = stats?.metrics ?? null;
  const st = store.data ?? null;
  const lanes = st ? Object.values(st).filter((l) => l.configured) : null;
  const ckpt = st?.["checkpoint"] ?? null;

  return (
    <div
      className="grid"
      style={{ gridTemplateColumns: "repeat(auto-fit, minmax(240px, 1fr))", gap: 12 }}
      role="list"
      aria-label="Product health summary"
    >
      <Card title="System" hover>
        {groupRows([
          { label: "engine", value: rt ? `${rt.engine.status} · ${rt.engine.run}` : conn.health ? "idle" : "unreachable" },
          { label: "api", value: `${conn.state}${conn.latencyMs !== null ? ` · ${fmtMs(conn.latencyMs)}` : ""}` },
          { label: "uptime", value: rt ? fmtMs(rt.engine.uptime_ms) : DASH },
        ])}
      </Card>
      <Card title="Execution" hover>
        {groupRows([
          { label: "jobs", value: m ? `${fmtNum(m.jobs_completed)} done · ${fmtNum(m.jobs_failed)} failed` : DASH },
          { label: "queue", value: rt ? `${fmtNum(rt.queue.depth)} waiting · ${fmtNum(rt.queue.inflight)} inflight` : DASH },
          { label: "workers", value: rt ? `${fmtNum(rt.workers.busy)} / ${fmtNum(rt.workers.registered)} busy` : DASH },
        ])}
      </Card>
      <Card title="Network" hover>
        {groupRows([
          { label: "requests", value: m ? `${fmtNum(m.api_requests_2xx)} 2xx · ${fmtNum(m.api_requests_4xx)} 4xx · ${fmtNum(m.api_requests_5xx)} 5xx` : DASH },
          { label: "retries", value: m ? `${fmtNum(m.requests_retry)} retried · ${fmtMs(m.backoff_sleep_ms_total)} backoff slept` : DASH },
          {
            label: "proxies",
            value: stats ? `${stats.proxies.length} egress(es)` : DASH,
            tone: m && m.api_requests_5xx > 0 ? "warn" : "ok",
          },
        ])}
      </Card>
      <Card title="Data" hover>
        {groupRows([
          { label: "records", value: m ? `${fmtNum(m.pages_fetched)} pages → ${fmtNum(m.records_extracted)} extracted → ${fmtNum(m.records_written)} written` : DASH },
          { label: "transforms", value: rt ? `${fmtNum(m ? m.requests_revalidated : 0)} revalidated · ${fmtNum(m ? m.storage_writes : 0)} writes` : DASH },
          { label: "schema", value: m ? `${fmtNum(m.schema_drifts)} drift events` : DASH, tone: m && m.schema_drifts > 0 ? "warn" : "ok" },
          {
            label: "quarantine",
            value: m ? `${fmtNum(m.records_quarantined)} diverted (never dropped silently)` : DASH,
            tone: m && m.records_quarantined > 0 ? "warn" : "idle",
          },
        ])}
      </Card>
      <Card title="Storage" hover>
        {groupRows([
          { label: "backends", value: lanes ? `${lanes.length} lane(s) configured` : st ? "none configured" : DASH },
          { label: "checkpoint", value: ckpt ? (ckpt.exists ? "present" : "configured, not yet written") : DASH },
          { label: "export", value: "CLI: harvest export --from <jsonl> --to <dir>" },
        ])}
      </Card>
      <Card title="Security" hover>
        {groupRows([
          {
            label: "auth",
            value: conn.tokenSet ? "bearer configured" : "loopback open",
            tone: conn.tokenSet ? "ok" : "idle",
          },
          {
            label: "secrets",
            value: conn.config ? (conn.config.admin_token === null ? "redacted in exports" : "exposed?!") : DASH,
            tone: conn.config && conn.config.admin_token === null ? "ok" : "idle",
          },
          {
            label: "audit",
            value: conn.config ? (conn.config.audit_path ? `audit: ${conn.config.audit_path}` : "audit lane off") : DASH,
          },
          {
            label: "attestation",
            value: conn.config ? (conn.config.attest_key_path ? "HMAC key configured" : "unsigned artifacts") : DASH,
            tone: conn.config?.attest_key_path ? "ok" : "idle",
          },
        ])}
      </Card>
    </div>
  );
}
