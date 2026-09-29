// Storage — split from Storage.tsx; barrel re-exports keep every public path & name

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtBytes, fmtNum } from "../../../lib/format";
import type { StorageResponse } from "../../../lib/types";
import { Card, PageHeader, StatusChip, MetricCard } from "../../../components/ui";
import { Page } from "../../../components/AppShell";
import { useConnection } from "../../../lib/connection";
import { Icon } from "../../../components/Icons";

export default function Storage() {
  const st = usePolled((s) => apiGet<StorageResponse>("/storage", { signal: s }), 10000);
  const conn = useConnection();
  const lanes = Object.entries(st.data ?? {});
  const cfg = conn.config;

  const bytes = lanes.reduce<number>((a, [, l]) => a + (l.bytes ?? 0), 0);
  const configuredCount = lanes.filter(([, l]) => l.configured).length;
  const missing = lanes.filter(([, l]) => l.configured && !l.exists);

  return (
    <Page>
      <PageHeader
        title="Storage Observatory"
        sub={`${configuredCount} of ${lanes.length} lanes configured · ${fmtBytes(bytes || null)} on disk`}
        actions={<StatusChip status={missing.length ? "degraded" : "ok"} label={missing.length ? `${missing.length} configured-but-missing` : "all present"} />}
      />

      <div className="grid grid-metrics" role="list" aria-label="Backend availability">
        <MetricCard label="Output backend" value={cfg ? String(cfg.output["kind"] ?? "—") : "—"} meta={<span className="t-caption">from GET /config → output.kind</span>} />
        <MetricCard label="SQLite build" value={cfg?._backends?.sqlite ? "compiled in" : "absent"} tone={cfg?._backends?.sqlite ? "ok" : "idle"} />
        <MetricCard label="PostgreSQL build" value={cfg?._backends?.postgres ? "compiled in" : "absent"} tone={cfg?._backends?.postgres ? "ok" : "idle"} meta={cfg?._backends?.postgres ? <span className="t-caption">pool state via writes; no probe endpoint</span> : undefined} />
        <MetricCard label="Redis build" value={cfg?._backends?.redis ? "compiled in" : "absent"} tone={cfg?._backends?.redis ? "ok" : "idle"} meta={<span className="t-caption">redis_url {cfg?.redis_url ? "configured" : "not set"}</span>} />
      </div>

      <Card title="Lanes">
        <table className="data">
          <thead>
            <tr>
              <th scope="col">Lane</th>
              <th scope="col">Kind</th>
              <th scope="col">Path (server-masked)</th>
              <th scope="col" className="num">Size</th>
              <th scope="col">State</th>
            </tr>
          </thead>
          <tbody>
            {lanes.map(([name, l]) => (
              <tr key={name}>
                <td style={{ textTransform: "capitalize" }}><Icon.Storage size={12} /> {name}</td>
                <td><code className="t-code">{l.kind}</code></td>
                <td><code className="t-code">{l.path || "—"}</code></td>
                <td className="num">{l.bytes === null ? "—" : fmtBytes(l.bytes)}</td>
                <td>
                  <StatusChip
                    status={l.configured ? (l.exists ? "ok" : "error") : "disabled"}
                    label={l.configured ? (l.exists ? "healthy" : "configured, not yet written") : "not configured"}
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>

      {cfg && (cfg.ckpt_path || cfg.revalidate_store || cfg.audit_path) && (
        <Card title="Durability settings">
          <dl className="kv" style={{ margin: 0 }}>
            <dt>Checkpoint cadence</dt>
            <dd>{fmtNum(cfg.ckpt_every_secs)}s ∥ {fmtNum(cfg.ckpt_every_pages)} pages — atomic temp+rename</dd>
            <dt>Resume</dt>
            <dd>CLI `harvest run --resume`; refused if policy sha changed</dd>
            <dt>Revalidation TTL</dt>
            <dd>{cfg.revalidate_ttl_secs ? `${fmtNum(cfg.revalidate_ttl_secs)}s` : "no TTL (re-crawl uses stored validators)"}</dd>
            <dt>Attestation</dt>
            <dd>{cfg.attest_key_path ? "sealed (key configured — value never shown)" : "unsigned"}</dd>
          </dl>
        </Card>
      )}
    </Page>
  );
}
