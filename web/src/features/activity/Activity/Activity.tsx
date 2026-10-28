// Activity — split from Activity.tsx; barrel re-exports keep every public path & name

import { useEffect, useMemo, useRef, useState } from "react";
import { apiGet } from "../../../lib/api";
import { EVENT_KINDS, type EventsResponse, type FailuresResponse } from "../../../lib/types";
import { Card, PageHeader, Segmented, StatusChip, EmptyState } from "../../../components/ui";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";
import { fmtClock, fmtBytes, fmtMs } from "../../../lib/format";
import { useConnection } from "../../../lib/connection";
import { KIND_TONE } from "./KIND_TONE";
import type { Filter } from "./Filter";
import { RetriesCard } from "./RetriesCard";


export default function Activity() {
  const conn = useConnection();
  const [filter, setFilter] = useState<Filter>("requests");
  const [live, setLive] = useState(true);
  const [since, setSince] = useState(0);
  const cursorRef = useRef(0);
  const [items, setItems] = useState<EventsResponse["events"]>([]);
  const [failures, setFailures] = useState<FailuresResponse | null>(null);

  useEffect(() => {
    let stop = false;
    let inFlight = false;
    const tick = async () => {
      if (inFlight) return;
      inFlight = true;
      if (document.visibilityState === "hidden" || !live) return;
      try {
        const kindQ = filter === "requests" ? "&kind=request_finished" : "";
        const d = await apiGet<EventsResponse>(`/events?limit=64&since_seq=${cursorRef.current}${kindQ}`);
        if (stop) return;
        if (d.events.length) {
          const nextCursor = Math.max(cursorRef.current, d.last_seq, ...d.events.map((e) => e.seq));
          cursorRef.current = nextCursor;
          setSince((current) => Math.max(current, nextCursor));
          setItems((current) => {
            const seen = new Set(current.map((event) => event.seq));
            const additions: EventsResponse["events"] = [];
            for (const event of d.events) {
              if (seen.has(event.seq)) continue;
              seen.add(event.seq);
              additions.push(event);
            }
            return additions.length ? [...additions, ...current].slice(0, 140) : current;
          });
        }
        const f = await apiGet<FailuresResponse>("/failures?limit=25");
        if (!stop) setFailures(f);
      } catch {
        /* offline is surfaced by the shell status bar; feed just pauses */
      } finally {
        inFlight = false;
      }
    };
    void tick();
    const h = window.setInterval(tick, 4000);
    return () => {
      stop = true;
      window.clearInterval(h);
    };
  }, [filter, live]);

  const shown = useMemo(() => {
    if (filter === "problems")
      return items.filter((e) => ["err", "warn"].includes(KIND_TONE[e.kind] ?? ""));
    return items;
  }, [items, filter]);

  return (
    <Page>
      <PageHeader
        title="Request Activity"
        sub={`ring ${conn.health ? "live" : "offline"} · cursor #${since} · newest first · delta polling`}
        actions={
          <>
            <button className="btn" onClick={() => setLive((v) => !v)} aria-pressed={!live}>
              {live ? <Icon.Pause size={12} /> : <Icon.Play size={12} />} {live ? "Pause" : "Resume"}
            </button>
            <Segmented
              ariaLabel="event filter"
              value={filter}
              onChange={setFilter}
              options={[
                { value: "requests", label: "Requests" },
                { value: "all", label: "All events" },
                { value: "problems", label: "Problems" },
              ]}
            />
          </>
        }
      />

      <div className="grid grid-2">
        <Card title="Live feed">
          {shown.length === 0 ? (
            <EmptyState
              icon={<Icon.Diagnostics size={18} />}
              title={conn.health?.running ? "No new events since cursor" : "No run is producing events"}
              body="The ring covers the live run. Start `harvest run`/`serve` and requests appear within one 4s delta poll."
            />
          ) : (
            <ol style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 2 }} aria-label="engine event feed">
              {shown.slice(0, 80).map((e) => {
                const tone = KIND_TONE[e.kind] ?? "idle";
                const d = e.detail;
                return (
                  <li key={e.seq} style={{ display: "flex", gap: 10, alignItems: "center", padding: "6px 4px", borderBottom: "1px solid var(--line-faint)", fontSize: 12.5 }}>
                    <i className={`dot ${tone}`} aria-hidden="true" />
                    <span className="t-code" style={{ color: "var(--text-3)", minWidth: 56 }}>{fmtClock(e.ts)}</span>
                    <span style={{ minWidth: 118, color: "var(--text-1)" }}>{e.kind.replaceAll("_", " ")}</span>
                    {typeof d["url"] === "string" && (
                      <code className="t-code" style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", flex: 1, minWidth: 0 }}>
                        {d["url"]}
                      </code>
                    )}
                    <span style={{ marginLeft: "auto", display: "inline-flex", gap: 8, fontFamily: "var(--font-mono)", fontVariantNumeric: "tabular-nums", color: "var(--text-2)" }}>
                      {typeof d["status"] === "number" && <StatusChip dot={false} status={d["status"] < 400 ? "ok" : d["status"] < 500 ? "degraded" : "error"} label={String(d["status"])} />}
                      {typeof d["ms"] === "number" && <span>{fmtMs(d["ms"])}</span>}
                      {typeof d["bytes"] === "number" && <span>{fmtBytes(d["bytes"])}</span>}
                      {typeof d["attempt"] === "number" && d["attempt"] > 1 && <span className="chip warn">a{d["attempt"]}</span>}
                    </span>
                  </li>
                );
              })}
            </ol>
          )}
        </Card>

        <div style={{ display: "grid", gap: "var(--s-4)", alignContent: "start" }}>
          <Card title="Recent failures">
            {failures?.items.length ? (
              <ul style={{ listStyle: "none", margin: 0, padding: 0, display: "grid", gap: 6 }}>
                {failures.items.map((f, i) => (
                  <li key={i} style={{ fontSize: 12.5 }}>
                    <code className="t-code">{String(f.url ?? "—")}</code>
                    <div className="t-caption">{String(f.error ?? f.status ?? "")}</div>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="t-caption">None in the live accounting window.</p>
            )}
          </Card>
          <RetriesCard />
          <Card title="Ring pressure">
            {eventsSummary()}
          </Card>
        </div>
      </div>
      <p className="t-caption">
        Kinds tracked: {EVENT_KINDS.length} engine events (source: ring bus).
        Attempt count and byte caps come from the same accounting the Prometheus metrics use.
      </p>
    </Page>
  );

  function eventsSummary() {
    const dropped = items.length ? shown.length : 0;
    return (
      <dl className="kv">
        <dt>captured (this view)</dt><dd>{dropped}</dd>
        <dt>poll cadence</dt><dd>4 s, cursor-gated</dd>
        <dt>pause</dt><dd>{live ? "resumable — paused" : "active"}</dd>
      </dl>
    );
  }
}
