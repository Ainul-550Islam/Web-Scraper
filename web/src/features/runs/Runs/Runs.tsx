// Runs — split from Runs.tsx; barrel re-exports keep every public path & name

import { useMemo, useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtDateTime, fmtHash, fmtMs, fmtNum } from "../../../lib/format";
import type { RecentRunRow, RunsResponse } from "../../../lib/types";
import { DataTable, type Column } from "../../../components/Table";
import { Button, Card, PageHeader, Segmented, StatusChip } from "../../../components/ui";
import { FilterBar, Tooltip } from "../../../components/patterns";
import { Icon } from "../../../components/Icons";
import { Page } from "../../../components/AppShell";
import type { SortKey } from "./SortKey";
import { toEpoch } from "./toEpoch";
import { RunDrawer } from "./RunDrawer";


/**
 * The table mirrors `GET /runs` exactly: server-side page window over the
 * newest 512 history lines, filters are real query parameters
 * (`status|sort|order|limit|offset|since|until`) — not client-side theater.
 */
export default function Runs() {
  const [status, setStatus] = useState<"all" | "ok" | "error" | "cancelled">("all");
  const [sort, setSort] = useState<SortKey>("ts");
  const [order, setOrder] = useState<"asc" | "desc">("desc");
  const [page, setPage] = useState(0);
  const [since, setSince] = useState("");
  const [until, setUntil] = useState("");
  const limit = 15;

  const qs = useMemo(() => {
    const p = new URLSearchParams({ limit: String(limit), offset: String(page * limit), sort, order });
    if (status !== "all") p.set("status", status);
    const s = toEpoch(since);
    const u = toEpoch(until);
    if (s !== null) p.set("since", String(s));
    if (u !== null) p.set("until", String(u + 86_399)); // inclusive day
    return p.toString();
  }, [status, sort, order, page, since, until]);

  const runs = usePolled((s) => apiGet<RunsResponse>(`/runs?${qs}`, { signal: s }), 10000, qs);
  const recent = usePolled((s) => apiGet<RecentRunRow[]>("/recent", { signal: s }), 10000);
  const [open, setOpen] = useState<RecentRunRow | null>(null);

  const rows = runs.data?.page.items ?? [];
  const detailFor = (run: string, ts: number, index: number) =>
    recent.data?.find((r) => r.run === run && r.ts === ts && r.index === index);

  const cols: Column<RunsResponse["page"]["items"][number]>[] = [
    { key: "index", label: "#", numeric: true, sortValue: (r) => r.index, render: (r) => r.index },
    { key: "ts", label: "Started", sortValue: (r) => r.ts, render: (r) => <Tooltip tip={`epoch ${r.ts}`}>{fmtDateTime(r.ts)}</Tooltip> },
    { key: "run", label: "Run", render: (r) => <code className="t-code">{r.run}</code> },
    { key: "status", label: "Status", render: (r) => <StatusChip status={r.status} /> },
    { key: "written", label: "Written", numeric: true, render: (r) => fmtNum(r.written) },
    { key: "quarantined", label: "Quarantined", numeric: true, hideable: true, defaultHidden: false, render: (r) => fmtNum(r.quarantined) },
    { key: "duration_ms", label: "Duration", numeric: true, render: (r) => fmtMs(r.duration_ms) },
    { key: "config_sha256", label: "Policy", hideable: true, render: (r) => <Tooltip tip={r.config_sha256}>{fmtHash(r.config_sha256)}</Tooltip> },
    { key: "error", label: "Error", hideable: true, render: (r) => r.error ? <span style={{ color: "var(--err)" }}>{r.error}</span> : "—" },
  ];

  return (
    <Page>
      <PageHeader
        title="Runs"
        sub="Paged view over the schedule history ledger"
      />
      <FilterBar>
        <Segmented
          ariaLabel="filter by status"
          value={status}
          onChange={(v) => { setStatus(v); setPage(0); }}
          options={[
            { value: "all", label: "All" },
            { value: "ok", label: "Ok" },
            { value: "error", label: "Errors" },
            { value: "cancelled", label: "Cancelled" },
          ]}
        />
        <span className="fb-group" role="group" aria-label="date range">
          <label className="t-caption" htmlFor="runs-since">from</label>
          <input id="runs-since" className="input" type="date" value={since} max={until || undefined} onChange={(e) => { setSince(e.target.value); setPage(0); }} />
          <label className="t-caption" htmlFor="runs-until">to</label>
          <input id="runs-until" className="input" type="date" value={until} min={since || undefined} onChange={(e) => { setUntil(e.target.value); setPage(0); }} />
          {(since || until) && (
            <Button variant="ghost" onClick={() => { setSince(""); setUntil(""); setPage(0); }}>
              clear dates
            </Button>
          )}
        </span>
        <Segmented
          ariaLabel="sort"
          value={sort}
          onChange={(v) => { setSort(v); setPage(0); }}
          options={[
            { value: "ts", label: "Time" },
            { value: "duration", label: "Duration" },
            { value: "written", label: "Written" },
          ]}
        />
        <Button onClick={() => setOrder(order === "desc" ? "asc" : "desc")}>
          <Icon.ChevronRight size={11} style={{ rotate: order === "desc" ? "90deg" : "-90deg" }} />
          {order}
        </Button>
        <span className="spacer" />
        <span className="t-caption">
          {runs.data ? `${runs.data.page.total} in window` : "loading…"} · newest {runs.data?.window ?? 0} ledger lines
        </span>
      </FilterBar>

      {runs.error && rows.length === 0 && runs.state === "offline" ? (
        <Card><div style={{ padding: 16 }}>History is read from `schedule.history_path` — start the engine to populate it.</div></Card>
      ) : (
        <DataTable
          caption="runs"
          columns={cols}
          rows={rows}
          rowKey={(r) => `${r.run}:${r.ts}:${r.index}`}
          total={runs.data?.page.total}
          limit={limit}
          offset={page * limit}
          onPage={(off) => setPage(Math.floor(off / limit))}
          loading={runs.state === "connecting"}
          error={runs.error}
          onRowClick={(r) => setOpen(detailFor(r.run, r.ts, r.index) ?? (r as RecentRunRow))}
          emptyTitle="No runs recorded yet"
          emptyBody={
            <>
              The ledger is <code>schedule.history_path</code> — each completed
              run appends one line. Once the engine finishes its first run,
              entries appear here.
            </>
          }
        />
      )}

      {open && <RunDrawer run={open} onClose={() => setOpen(null)} />}
    </Page>
  );
}
