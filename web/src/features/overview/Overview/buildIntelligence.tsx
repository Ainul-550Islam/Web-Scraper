// buildIntelligence — split from Overview.tsx; barrel re-exports keep every public path & name

import type { useConnection } from "../../../lib/connection";
import { fmtMs, fmtNum, fmtPct } from "../../../lib/format";
import type { RuntimeSnapshot, StatsResponse } from "../../../lib/types";
import type { IntelligenceRow } from "./IntelligenceRow";


export function buildIntelligence(
  config: ReturnType<typeof useConnection>["config"],
  rt: RuntimeSnapshot | null,
  m: StatsResponse["metrics"] | null,
  alerts: { fired: number; notified: number; items: unknown[] } | null,
  success: number | null,
  running: boolean,
): IntelligenceRow[] {
  const toneOf = (bad: number, warn: number, err: number): IntelligenceRow["tone"] =>
    Number.isNaN(bad) ? "idle" : bad >= err ? "err" : bad >= warn ? "warn" : "ok";

  return [
    {
      label: "Current state",
      tone: running ? "ok" : "info",
      lines: [
        running
          ? `A crawl run is active${rt ? ` — “${rt.engine.run}”, up ${fmtMs(rt.engine.uptime_ms)}` : ""}.`
          : `No run active.${config ? ` The scheduler fires every ${config.schedule.every_secs}s.` : ""}`,
        config
          ? `Server-side validation: ${config.validation}${config._warnings.length ? ` · ${config._warnings.length} warning(s)` : ""}.`
          : "No config attached (server built without serve?).",
      ],
    },
    {
      label: "Throughput",
      tone: "info",
      lines: rt
        ? [
            `${fmtNum(rt.accounting.jobs_per_sec)} jobs/s · ${fmtNum(rt.accounting.requests)} requests this run · ${fmtNum(m?.pages_fetched ?? 0)} pages fetched total.`,
            `Avg queue wait ${fmtMs(rt.accounting.avg_queue_wait_ms || null)} · politeness sleeps ${fmtNum(m?.politeness_sleeps ?? 0)} (pacing cost, counted).`,
          ]
        : ["Attach a run to see live throughput; totals still come from /stats."],
    },
    {
      label: "Reliability",
      tone: success === null ? "idle" : toneOf((1 - success) * 100, 2, 10),
      lines: [
        success !== null
          ? `${fmtPct(success)} request success${m && m.requests_retry ? `, ${fmtNum(m.requests_retry)} retries absorbed` : " with zero retries"} (process lifetime).`
          : "No request data yet.",
        m
          ? `Refusals blocked: ${fmtNum(m.requests_blocked)} · breaker rejections ${fmtNum(m.breaker_rejected)} · domains abandoned ${fmtNum(m.domains_abandoned)}.`
          : "",
      ].filter(Boolean),
    },
    {
      label: "Data quality",
      tone: toneOf((m?.records_quarantined ?? 0) + (m?.schema_drifts ?? 0), 1, 50),
      lines: m
        ? [
            `Extracted ${fmtNum(m.records_extracted)} · deduped-out ${fmtNum(m.records_deduped)} · written ${fmtNum(m.records_written)}.`,
            `Quarantined ${fmtNum(m.records_quarantined)} · schema drifts ${fmtNum(m.schema_drifts)} · pipeline failures ${fmtNum(m.pipeline_failures)}.`,
          ]
        : ["Waiting for the first records."],
    },
    {
      label: "Workers & queue",
      tone: toneOf(rt?.workers.deaths ?? 0, 1, 3),
      lines: rt
        ? [
            `${fmtNum(rt.workers.registered)} registered · utilization ${fmtPct(rt.workers.utilization)} · deaths ${fmtNum(rt.workers.deaths)} (supervised: +${fmtNum(rt.workers.total_restarts)} restarts).`,
            `Queue depth ${fmtNum(rt.queue.depth)}, inflight ${fmtNum(rt.queue.inflight)}, rejected ${fmtNum(rt.queue.rejected)} (max ${rt.queue.max_depth === 0 ? "unbounded" : fmtNum(rt.queue.max_depth)}).`,
          ]
        : ["Queue and worker state is live-run state; between runs there is nothing to pool."],
    },
    {
      label: "Alerts & storage",
      tone: alerts && alerts.items.length ? "warn" : "ok",
      lines: [
        alerts
          ? `${fmtNum(alerts.fired)} alerts fired · ${fmtNum(alerts.notified)} notified · ${alerts.items.length} live entr${alerts.items.length === 1 ? "y" : "ies"}.`
          : "Alert state unread.",
        m
          ? `${fmtNum(m.checkpoints_written)} checkpoints · ${fmtNum(m.storage_writes)} storage writes.`
          : "",
      ].filter(Boolean),
    },
  ];
}
