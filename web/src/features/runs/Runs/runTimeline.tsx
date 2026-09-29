// runTimeline — split from Runs.tsx; barrel re-exports keep every public path & name

import { fmtDateTime, fmtHash, fmtMs, fmtNum } from "../../../lib/format";
import type { RecentRunRow } from "../../../lib/types";
import { type TimelineItem } from "../../../components/patterns";


export function runTimeline(run: RecentRunRow): TimelineItem[] {
  const s = run.summary;
  return [
    { title: "run accepted", tone: "ok", meta: <>policy sha {fmtHash(run.config_sha256, 6)} · {fmtDateTime(run.ts)}</> },
    {
      title: "crawl executed",
      tone: "info",
      meta: s
        ? <>{fmtNum(s.pages_fetched)} pages · {fmtNum(s.retries)} retries · {fmtNum(s.blocked)} blocked · {fmtNum(s.robots_denied)} robots denials</>
        : <>no summary block on this row</>,
    },
    {
      title: run.status === "ok" ? "completed" : run.status === "cancelled" ? "cancelled" : "finished with error",
      tone: run.status === "ok" ? "ok" : "err",
      meta: <>{fmtNum(run.written)} written · {fmtNum(run.quarantined)} quarantined · {fmtMs(run.duration_ms)} elapsed</>,
    },
    ...(s
      ? [{
          title: "export & audit",
          tone: (Number(s["schema_drifts"] ?? 0) > 0 || Number(s["quarantined"] ?? 0) > 0 ? "warn" : "") as TimelineItem["tone"],
          meta: (
            <>
              {s.ckpt_writes > 0 ? `${fmtNum(s.ckpt_writes)} checkpoints · ` : ""}
              {fmtNum(s.deduped)} deduped out · {fmtNum(Number(s["schema_drifts"] ?? 0))} schema drifts · manifest verification runs from
              the CLI (<code className="t-code">harvest verify</code>)
            </>
          ),
        }]
      : []),
  ];
}
