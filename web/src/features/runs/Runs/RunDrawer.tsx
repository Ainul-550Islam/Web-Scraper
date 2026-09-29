// RunDrawer — split from Runs.tsx; barrel re-exports keep every public path & name

import { fmtDateTime, fmtHash, fmtMs, fmtNum } from "../../../lib/format";
import type { RecentRunRow } from "../../../lib/types";
import { Card, StatusChip } from "../../../components/ui";
import { Drawer } from "../../../components/Overlays";
import { Timeline, Tooltip } from "../../../components/patterns";
import { runTimeline } from "./runTimeline";


export function RunDrawer({ run, onClose }: { run: RecentRunRow; onClose: () => void }) {
  const per = run.summary?.per_target ?? {};
  return (
    <Drawer
      title={<span>Run <code className="t-code">{run.run}</code> · <StatusChip status={run.status} /></span>}
      onClose={onClose}
    >
      <dl className="kv">
        <dt>Started</dt><dd>{fmtDateTime(run.ts)}</dd>
        <dt>Duration</dt><dd>{fmtMs(run.duration_ms)}</dd>
        <dt>Written</dt><dd>{fmtNum(run.written)}</dd>
        <dt>Quarantined</dt><dd>{fmtNum(run.quarantined)}</dd>
        <dt>Config sha</dt><dd><Tooltip tip={run.config_sha256}><code className="t-code">{fmtHash(run.config_sha256, 12)}…</code></Tooltip></dd>
        {run.summary && (
          <>
            <dt>Pages fetched</dt><dd>{fmtNum(run.summary.pages_fetched)}</dd>
            <dt>Retries</dt><dd>{fmtNum(run.summary.retries)}</dd>
            <dt>Blocked</dt><dd>{fmtNum(run.summary.blocked)}</dd>
            <dt>robots denied</dt><dd>{fmtNum(run.summary.robots_denied)}</dd>
            <dt>deduped out</dt><dd>{fmtNum(run.summary.deduped)}</dd>
            <dt>checkpoint writes</dt><dd>{fmtNum(run.summary.ckpt_writes)}</dd>
            <dt>schema drifts</dt><dd>{fmtNum(Number(run.summary["schema_drifts"] ?? 0))}</dd>
            <dt>respect_robots</dt><dd>{String(run.respect_robots)}</dd>
          </>
        )}
        {run.error && (<><dt>Error</dt><dd style={{ color: "var(--err)" }}>{run.error}</dd></>)}
      </dl>
      {Object.keys(per).length > 0 && (
        <Card title="Per target">
          <table className="data">
            <thead><tr><th scope="col">Target</th><th scope="col">Written</th><th scope="col">Blocked</th><th scope="col">Failed</th><th scope="col">Quarantined</th><th scope="col">Revalidated</th></tr></thead>
            <tbody>
              {Object.entries(per).map(([t, v]) => (
                <tr key={t}>
                  <td data-col="Target"><code className="t-code">{t}</code></td>
                  <td data-col="Written" className="num">{fmtNum(v.written)}</td>
                  <td data-col="Blocked" className="num">{fmtNum(v.blocked)}</td>
                  <td data-col="Failed" className="num">{fmtNum(v.failed)}</td>
                  <td data-col="Quarantined" className="num">{fmtNum(v.quarantined)}</td>
                  <td data-col="Revalidated" className="num">{fmtNum(v.revalidated)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {run.summary ? (
        <Card title="Lifecycle">
          <Timeline label="run lifecycle" items={runTimeline(run)} />
        </Card>
      ) : (
        <p className="t-caption">
          Only the flat RunInfo row is available for this entry ({fmtNum(run.written)} written) —
          the summary block is present on rows written by recent engine versions.
        </p>
      )}
    </Drawer>
  );
}
