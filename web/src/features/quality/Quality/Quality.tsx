// Quality — split from Quality.tsx; barrel re-exports keep every public path & name

import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtNum } from "../../../lib/format";
import type { PipelineResponse, StatsResponse } from "../../../lib/types";
import { Card, MetricCard, PageHeader, StatusChip } from "../../../components/ui";
import { Page } from "../../../components/AppShell";

export default function Quality() {
  const stats = usePolled((s) => apiGet<StatsResponse>("/stats", { signal: s }), 8000);
  const pipe = usePolled((s) => apiGet<PipelineResponse>("/pipeline", { signal: s }), 8000);
  const m = stats.data?.metrics;

  if (!m) {
    return (
      <Page>
        <PageHeader title="Data Quality" sub="waiting for /stats…" />
        <Card><p className="t-caption">Counters become available when the admin API answers.</p></Card>
      </Page>
    );
  }

  const steps: { label: string; value: number; note: string }[] = [
    { label: "Requests issued", value: m.requests_total, note: `${fmtNum(m.requests_ok)} ok · ${fmtNum(m.requests_retry)} retried · ${fmtNum(m.requests_blocked)} blocked` },
    { label: "Pages fetched", value: m.pages_fetched, note: `${fmtNum(m.requests_revalidated)} answered with 304 (cheap re-crawls)` },
    { label: "Records extracted", value: m.records_extracted, note: "extraction pipeline output, pre-dedup" },
    { label: "Duplicates dropped", value: -m.records_deduped, note: "content fingerprints already seen" },
    { label: "Records written", value: m.records_written, note: "landed in the configured sink" },
  ];
  const losses = [
    { label: "Quarantined (schema violations)", value: m.records_quarantined, tone: "warn" as const },
    { label: "Schema drift events", value: m.schema_drifts, tone: "warn" as const },
    { label: "Pipeline failures", value: m.pipeline_failures, tone: "err" as const },
    { label: "robots denials", value: m.robots_denials, tone: "info" as const },
    { label: "domains abandoned (breaker)", value: m.domains_abandoned, tone: "err" as const },
    { label: "budget refusals (records)", value: m.budget_denied_records, tone: "idle" as const },
  ];
  const max = Math.max(m.requests_total, 1);

  const writtenPct = m.requests_total ? m.records_written / max : 0;

  return (
    <Page>
      <PageHeader
        title="Data Quality"
        sub="process-lifetime counters; the same numbers Prometheus exports"
        actions={
          <StatusChip
            status={m.pipeline_failures + m.records_quarantined > 0 ? "degraded" : "ok"}
            label={m.pipeline_failures + m.records_quarantined > 0 ? "attention" : "clean"}
          />
        }
      />

      <div className="grid grid-metrics" role="list" aria-label="Quality summary">
        <MetricCard label="Written" value={fmtNum(m.records_written)} tone="ok" meta={<>{(writtenPct * 100).toFixed(1)}% of request volume</>} />
        <MetricCard label="Rejected (quarantine)" value={fmtNum(m.records_quarantined)} tone={m.records_quarantined ? "warn" : "idle"} meta={<span className="t-caption">strict-mode schema violations are diverted, not dropped</span>} />
        <MetricCard label="Deduped out" value={fmtNum(m.records_deduped)} meta={<span className="t-caption">exact-repeat suppression</span>} />
        <MetricCard label="Drift events" value={fmtNum(m.schema_drifts)} tone={m.schema_drifts ? "warn" : "idle"} meta={<span className="t-caption">field-type changes vs. declared schema</span>} />
      </div>

      <Card title="Pipeline funnel (sequential stages)">
        <div className="funnel">
          {steps.map((s) => {
            const neg = s.value < 0;
            const w = Math.max(1.5, (Math.abs(s.value) / max) * 100);
            return (
              <div className="funnel-row" key={s.label}>
                <span>{s.label}</span>
                <span className={`funnel-bar ${neg ? "loss" : ""}`} style={{ width: `${w}%` }} aria-hidden="true" />
                <span className="funnel-val" title={s.note}>{neg ? `−${fmtNum(-s.value)}` : fmtNum(s.value)}</span>
              </div>
            );
          })}
        </div>
        <p className="t-caption" style={{ marginTop: 8 }}>
          Widths are against `requests_total`; hover a number for its meaning.
          {pipe.data ? ` Last run: ${fmtNum(pipe.data.written)} written / ${fmtNum(pipe.data.quarantined)} quarantined.` : ""}
        </p>
      </Card>

      <Card title="Loss accounting">
        <table className="data">
          <thead><tr><th scope="col">Signal</th><th scope="col" className="num">Count</th><th scope="col">Tone</th></tr></thead>
          <tbody>
            {losses.map((l) => (
              <tr key={l.label}>
                <td data-col="Signal">{l.label}</td>
                <td className="num">{fmtNum(l.value)}</td>
                <td><StatusChip status={l.value > 0 ? l.tone : "disabled"} label={l.value > 0 ? "present" : "none"} /></td>
              </tr>
            ))}
          </tbody>
        </table>
      </Card>
    </Page>
  );
}
