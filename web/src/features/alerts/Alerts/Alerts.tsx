// Alerts — split from Alerts.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { apiGet, apiPost, invalidateCache } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtAgo, fmtNum } from "../../../lib/format";
import type { AlertsResponse } from "../../../lib/types";
import { Button, Card, EmptyState, PageHeader, StatusChip } from "../../../components/ui";
import { ConfirmModal } from "../../../components/Overlays";
import { Page } from "../../../components/AppShell";
import { useToasts } from "../../../hooks/useToasts";
import { useConnection } from "../../../lib/connection";
import { Icon } from "../../../components/Icons";

export default function Alerts() {
  const conn = useConnection();
  const alerts = usePolled((s) => apiGet<AlertsResponse>("/alerts", { signal: s }), 7000);
  const { push } = useToasts();
  const [confirm, setConfirm] = useState<null | string | "*">(null);
  const [busy, setBusy] = useState(false);
  const a = alerts.data ?? conn.alerts;

  const ack = async (event: string) => {
    // Loopback binds accept unauthenticated posts; anything else answers 401
    // and the toast below surfaces the server's own message.
    setBusy(true);
    try {
      const res = await apiPost<{ acked: number; result: string }>(
        `/alerts/ack?event=${encodeURIComponent(event)}`,
        {},
      );
      invalidateCache();
      push("ok", res.result === "no-live-entry" ? `Nothing live to acknowledge for “${event}”` : `${fmtNum(res.acked)} entr${res.acked === 1 ? "y" : "ies"} acknowledged`);
    } catch (e) {
      push("err", e instanceof Error ? e.message : "ack failed");
      throw e;
    } finally {
      setBusy(false);
    }
  };

  const items = a?.items ?? [];

  return (
    <Page>
      <PageHeader
        title="Alerts"
        sub={
          a
            ? <>{fmtNum(a.fired)} fired · {fmtNum(a.notified)} notified · cooldown {fmtNum(a.cooldown_secs)}s {a.min_severity ? <> · min severity <strong>{a.min_severity}</strong></> : null}</>
            : "slo + lifecycle alerts with suppression"
        }
        actions={
          items.length > 0 ? (
            <Button variant="danger" onClick={() => setConfirm("*")} disabled={busy}>
              Acknowledge all
            </Button>
          ) : undefined
        }
      />

      <Card title="Live alert entries">
        {items.length === 0 ? (
          <EmptyState
            icon={<Icon.Alerts size={18} />}
            title="No live alerts"
            body={
              conn.config
                ? <>Alerts fire when <code className="t-code">alerts</code> thresholds break (min_records {fmtNum(conn.config.alerts.min_records)}, max_block_rate {conn.config.alerts.max_block_rate}, quarantine ≥ {fmtNum(conn.config.alerts.quarantine_threshold)}) and are suppressed during the {fmtNum(a?.cooldown_secs ?? 0)}s cooldown.</>
                : "The alerts view tracks the engine's in-memory alert state."
            }
          />
        ) : (
          <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 6 }}>
            {items.map((al, i) => {
              const key = String(al["key"] ?? al["id"] ?? al["kind"] ?? `entry-${i}`);
              const sev = String(al["severity"] ?? "warning");
              return (
                <li key={i} style={{ display: "flex", gap: 12, alignItems: "center", padding: "8px 4px", borderBottom: "1px solid var(--line-faint)" }}>
                  <StatusChip status={sev === "critical" ? "error" : "degraded"} label={sev} />
                  <span style={{ minWidth: 0 }}>
                    <strong style={{ fontSize: 13 }}>{String(al["message"] ?? key)}</strong>
                    <div className="t-caption">{key} · {fmtAgo(Number(al["ts"] ?? 0))}</div>
                  </span>
                  <span style={{ marginLeft: "auto", display: "flex", gap: 6 }}>
                    <Button className="sm" disabled={busy} onClick={() => setConfirm(key)}>Acknowledge</Button>
                  </span>
                </li>
              );
            })}
          </ul>
        )}
      </Card>

      <Card title="Suppression & delivery">
        <dl className="kv" style={{ margin: 0 }}>
          <dt>Cooldown window</dt><dd>{fmtNum(a?.cooldown_secs)} s per alert key</dd>
          <dt>Notifier</dt><dd>{conn.config?.alerts.webhook_url ? "webhook configured (URL never sent to the browser)" : "not configured — alerts stay local"}</dd>
          <dt>Notified vs fired</dt><dd>{fmtNum(a?.notified)} / {fmtNum(a?.fired)} (delta = cooldown-suppressed)</dd>
        </dl>
      </Card>

      {confirm !== null && (
        <ConfirmModal
          title={confirm === "*" ? "Acknowledge ALL alerts" : `Acknowledge “${confirm}”`}
          danger={confirm === "*"}
          confirmLabel="Acknowledge"
          onClose={() => setConfirm(null)}
          onConfirm={async () => { await ack(confirm); }}
          body={
            confirm === "*"
              ? <>This acknowledges <strong>every live alert entry</strong> on this engine. The action is recorded in the ops feed and the audit journal with a correlation id.</>
              : "Acknowledging removes the entry from the live alert set. It is recorded in the ops feed."
          }
        />
      )}
    </Page>
  );
}
