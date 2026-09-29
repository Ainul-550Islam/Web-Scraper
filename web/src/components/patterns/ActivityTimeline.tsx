/**
 * ActivityTimeline — the engine event ring rendered through the existing
 * `Timeline` pattern, bounded to the newest `cap` (default 60) events.
 *
 * The event vocabulary here is exactly what `GET /events` emits
 * (`src/lib/types/EVENT_KINDS.ts` — one entry per engine `EventKind`);
 * kinds are mapped to tones, never invented. When the ring says events were
 * dropped, the caption says so instead of pretending the tail is complete.
 */
import type { EngineEvent } from "../../lib/types";
import { fmtClock } from "../../lib/format";
import { Timeline } from "./Timeline";
import type { TimelineItem } from "./TimelineItem";

const ERR_KINDS = new Set(["job_failed", "breaker_opened", "queue_overflow", "proxy_failure", "budget_exhausted"]);
const OK_KINDS = new Set([
  "job_completed",
  "request_finished",
  "sink_write_completed",
  "extraction_completed",
  "breaker_closed",
  "shutdown_completed",
]);
const WARN_KINDS = new Set(["job_expired", "retry_scheduled", "record_quarantined"]);

function toneForKind(kind: string): TimelineItem["tone"] {
  if (ERR_KINDS.has(kind)) return "err";
  if (WARN_KINDS.has(kind)) return "warn";
  if (OK_KINDS.has(kind)) return "ok";
  return "info";
}

export interface ActivityTimelineProps {
  events: EngineEvent[] | null | undefined;
  /** newest-first slice size (default 60, mirroring the server's own caps) */
  cap?: number;
  /** ring drops reported by GET /events (`dropped`) */
  dropped?: number;
  label?: string;
}

function eventTimelineItems(events: EngineEvent[]): TimelineItem[] {
  return events.map((e) => ({
    title: e.kind,
    meta: (
      <>
        {fmtClock(e.ts)}
        {e.job !== null && e.job !== undefined ? ` · job ${e.job}` : ""}
        {typeof e.detail?.url === "string" ? ` · ${e.detail.url}` : ""}
      </>
    ),
    tone: toneForKind(e.kind),
  }));
}

export function ActivityTimeline({ events, cap = 60, dropped, label }: ActivityTimelineProps) {
  const list = events ?? [];
  const shown = list.slice(0, cap);
  const hidden = Math.max(0, list.length - shown.length);
  return (
    <>
      <Timeline items={eventTimelineItems(shown)} label={label ?? "recent engine events"} />
      {(hidden > 0 || (dropped ?? 0) > 0) && (
        <p className="t-caption">
          {hidden > 0 && `newest ${cap} shown — ${hidden} more in the response. `}
          {dropped !== undefined && dropped > 0 && `Ring dropped ${dropped} event(s) under load.`}
        </p>
      )}
    </>
  );
}
