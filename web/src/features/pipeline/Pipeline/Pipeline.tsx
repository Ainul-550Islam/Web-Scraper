// Pipeline — split from Pipeline.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtNum } from "../../../lib/format";
import type { PipelineResponse } from "../../../lib/types";
import { Card, PageHeader, StatusChip } from "../../../components/ui";
import { PipelineConnector, PipelineNode } from "../../../components/patterns";
import { Page } from "../../../components/AppShell";
import { useConnection } from "../../../lib/connection";
import type { Ctx } from "./Ctx";
import { STAGES } from "./STAGES";
import { stageTone } from "./stageTone";
import { renderDetail } from "./renderDetail";


export default function Pipeline() {
  const conn = useConnection();
  const pipe = usePolled((s) => apiGet<PipelineResponse>("/pipeline", { signal: s }), 8000);
  const stats = usePolled((s) => apiGet<{ metrics: Record<string, number> }>("/stats", { signal: s }), 8000);
  const [selected, setSelected] = useState<string | null>(null);

  const stages: Record<string, number> = {
    requests_total: stats.data?.metrics["requests_total"] ?? 0,
    requests_retry: stats.data?.metrics["requests_retry"] ?? 0,
    records_extracted: stats.data?.metrics["records_extracted"] ?? 0,
    records_deduped: stats.data?.metrics["records_deduped"] ?? 0,
    records_written: stats.data?.metrics["records_written"] ?? 0,
    schema_drifts: stats.data?.metrics["schema_drifts"] ?? 0,
    transform_ops: Math.max(0, ...(pipe.data?.per_target ?? []).map((t) => t.transform_ops)),
    schema_fields: Math.max(0, ...(pipe.data?.per_target ?? []).map((t) => t.schema_fields)),
  };
  const cfg = conn.config;
  const ctx: Ctx = {
    pipe: pipe.data ?? null,
    stages,
    targets: cfg?.targets.length ?? 0,
    policy: cfg?.default_policy
      ? { rps: cfg.default_policy.requests_per_sec, burst: cfg.default_policy.burst, minInterval: cfg.default_policy.min_interval_ms }
      : null,
    proxies: stats.data ? 1 : 0,
    sinks: cfg ? `${String(cfg.output.kind)} sink` : "—",
    selected,
  };

  return (
    <Page>
      <PageHeader
        title="Pipeline"
        sub="the real engine chain — read-only, every stage annotated with its live source"
        actions={pipe.data && <StatusChip status={pipe.data.pipeline_failures ? "degraded" : "ok"} label={pipe.data.pipeline_failures ? `${pipe.data.pipeline_failures} pipeline failures` : "flowing"} />}
      />

      <Card title="Stage graph">
        <div className="pipe" role="list" aria-label="pipeline stages">
          {STAGES.map((st, i) => (
            <span key={st.id} style={{ display: "contents" }} role="listitem">
              <PipelineNode
                title={st.title}
                tone={stageTone(st.id, ctx)}
                meta={st.meta(ctx)[0]}
                sub={st.meta(ctx)[1]}
                selected={selected === st.id}
                onSelect={() => setSelected(selected === st.id ? null : st.id)}
              />
              {i < STAGES.length - 1 && <PipelineConnector />}
            </span>
          ))}
        </div>
      </Card>

      {selected && (
        <Card title={`Stage detail · ${STAGES.find((s) => s.id === selected)?.title}`} foot={<span className="t-caption">source: {STAGES.find((s) => s.id === selected)?.source}</span>}>
          {renderDetail(selected, ctx)}
        </Card>
      )}

      <Card title="Per-target pipeline contract (GET /pipeline)">
        {pipe.data?.per_target.length ? (
          <table className="data">
            <thead><tr><th scope="col">Target</th><th scope="col">Schema enforce</th><th scope="col">Declared fields</th><th scope="col">Transform ops</th></tr></thead>
            <tbody>
              {pipe.data.per_target.map((t) => (
                <tr key={t.target}>
                  <td><code className="t-code">{t.target}</code></td>
                  <td><StatusChip dot={false} status={t.enforce === "strict" ? "ok" : t.enforce === "coerce" ? "degraded" : "idle"} label={t.enforce} /></td>
                  <td className="num">{fmtNum(t.schema_fields)}</td>
                  <td className="num">{fmtNum(t.transform_ops)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        ) : (
          <p className="t-caption">No pipeline data — the endpoint reflects the latest run's contract tally.</p>
        )}
        {pipe.data && (
          <div className="kv" style={{ marginTop: 10 }}>
            <dt>written</dt><dd>{fmtNum(pipe.data.written)}</dd>
            <dt>quarantined</dt><dd>{fmtNum(pipe.data.quarantined)}</dd>
            <dt>schema drifts</dt><dd>{fmtNum(pipe.data.schema_drifts)}</dd>
            <dt>pipeline failures</dt><dd>{fmtNum(pipe.data.pipeline_failures)}</dd>
          </div>
        )}
      </Card>
    </Page>
  );
}
