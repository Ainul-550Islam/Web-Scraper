/**
 * Event Stream — the structured engine ring over `GET /events`.
 *
 * Server-side parameters are exactly what the route accepts (see
 * `docs/api/05-get-events-structured-event-tail.md`): `limit`, `kind`,
 * `since_seq`. Everything else — the URL substring narrowing and the row
 * selection — is client-side over the fetched window and is labeled as such.
 * Selected event bodies render through `JsonInspector` (redaction-first),
 * never through a raw <pre> of unmasked data.
 */
import { useMemo, useState } from "react";
import { apiGet } from "../../lib/api";
import { usePolled } from "../../hooks/usePolled";
import { EVENT_KINDS, type EngineEvent, type EventsResponse } from "../../lib/types";
import { fmtClock, fmtNum } from "../../lib/format";
import { Page } from "../../components/AppShell";
import { Card, EmptyState, Field, PageHeader, Segmented, SelectInput, StatusChip, TextInput } from "../../components/ui";
import { ActivityTimeline } from "../../components/patterns/ActivityTimeline";
import { FilterBar } from "../../components/patterns";
import { JsonInspector } from "../../components/patterns/JsonInspector";

const SIZES = [50, 200] as const;

function toneFor(kind: EngineEvent["kind"]): string {
  if (kind.includes("failed") || kind === "breaker_opened" || kind === "queue_overflow" || kind === "proxy_failure" || kind === "budget_exhausted")
    return "error";
  if (kind === "retry_scheduled" || kind === "record_quarantined" || kind === "job_expired") return "degraded";
  if (kind.endsWith("completed") || kind === "breaker_closed") return "ok";
  return "info";
}

export default function EventsPage() {
  const [limit, setLimit] = useState<(typeof SIZES)[number]>(50);
  const [kind, setKind] = useState<string>("all");
  const [sinceSeq, setSinceSeq] = useState("");
  const [contains, setContains] = useState("");
  const [selected, setSelected] = useState<EngineEvent | null>(null);

  const qs = useMemo(() => {
    const p = new URLSearchParams({ limit: String(limit) });
    if (kind !== "all") p.set("kind", kind);
    const n = Number(sinceSeq);
    if (sinceSeq.trim() !== "" && Number.isInteger(n) && n >= 0) p.set("since_seq", String(n));
    return p.toString();
  }, [limit, kind, sinceSeq]);

  const ev = usePolled((s) => apiGet<EventsResponse>(`/events?${qs}`, { signal: s }), 5000, qs);
  const data = ev.data;

  const rows = useMemo(() => {
    let list = data?.events ?? [];
    const needle = contains.trim().toLowerCase();
    if (needle !== "") {
      // client-side narrowing of the fetched window only — the server does
      // not offer substring search on /events
      list = list.filter((e) => JSON.stringify(e).toLowerCase().includes(needle));
    }
    return list;
  }, [data, contains]);

  return (
    <Page>
      <PageHeader
        title="Event Stream"
        sub={
          data
            ? `ring ${fmtNum(data.ring_len)} · last seq ${fmtNum(data.last_seq)} · dropped ${fmtNum(data.dropped)}`
            : ev.error
              ? "events endpoint unreachable"
              : "loading…"
        }
      />

      <FilterBar>
        <Segmented
          ariaLabel="Fetch window"
          value={String(limit)}
          onChange={(v) => setLimit(Number(v) as (typeof SIZES)[number])}
          options={SIZES.map((n) => ({ label: `last ${n}`, value: String(n) }))}
        />
        <Field label="kind">
          <SelectInput value={kind} onChange={(ev) => setKind(ev.target.value)} aria-label="event kind">
            <option value="all">all kinds</option>
            {EVENT_KINDS.map((k) => (
              <option key={k} value={k}>{k}</option>
            ))}
          </SelectInput>
        </Field>
        <Field label="since_seq ≥">
          <TextInput value={sinceSeq} onChange={(ev) => setSinceSeq(ev.target.value)} placeholder="e.g. 1402" aria-label="since sequence" />
        </Field>
        <Field label="contains (this window only)">
          <TextInput value={contains} onChange={(ev) => setContains(ev.target.value)} placeholder="url or text in the fetched tail" aria-label="substring filter over fetched window" />
        </Field>
        <span className="spacer" />
        <StatusChip status={data && data.dropped > 0 ? "degraded" : "ok"} label={data && data.dropped > 0 ? `ring dropped ${data.dropped}` : "ring intact"} dot={false} />
      </FilterBar>

      <div className="grid grid-2">
        <Card title="Tail" hover>
          {rows.length === 0 ? (
            <EmptyState
              title={ev.error ? "No events available" : "Nothing in this window"}
              body={
                ev.error
                  ? "GET /events did not answer (engine down or token missing)."
                  : contains.trim() !== ""
                    ? "No fetched event matches the filter — widen the window or clear the text filter."
                    : "The ring is empty until a run publishes."
              }
            />
          ) : (
            <>
              <ActivityTimeline events={rows} cap={60} label="event tail" />
              <ul className="ev-rows" role="list">
                {rows.map((e) => (
                  <li key={e.seq}>
                    <button type="button" className={`ev-row ${selected?.seq === e.seq ? "is-open" : ""}`} onClick={() => setSelected(selected?.seq === e.seq ? null : e)}>
                      <StatusChip status={toneFor(e.kind)} dot label={`#${e.seq} ${e.kind}`} />
                      <span className="t-muted">{fmtClock(e.ts)}</span>
                      {typeof e.detail?.url === "string" && <span className="ev-url">{e.detail.url}</span>}
                    </button>
                  </li>
                ))}
              </ul>
            </>
          )}
        </Card>
        <div>
          {selected ? (
            <>
              <JsonInspector value={selected} />
              <p className="t-caption t-muted">
                Displayed and copied output is redaction-masked (`src/lib/redaction.ts`) before render —
                the raw response never reaches the DOM unmasked.
              </p>
            </>
          ) : (
            <EmptyState title="Select an event" body="Pick a row on the left to inspect its structured detail payload." />
          )}
        </div>
      </div>
      <style>{`
        .ev-rows { display: flex; flex-direction: column; gap: 4px; margin-top: 10px; }
        .ev-row  { align-items: center; background: none; border: 1px solid transparent; border-radius: 8px;
                   color: inherit; cursor: pointer; display: flex; font: inherit; gap: 10px; padding: 6px 8px;
                   text-align: left; width: 100%; }
        .ev-row:hover { border-color: var(--line, currentColor); }
        .ev-row.is-open { background: color-mix(in srgb, currentColor 8%, transparent); }
        .ev-url  { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
      `}</style>
    </Page>
  );
}
