// Audit — split from Audit.tsx; barrel re-exports keep every public path & name

import { useMemo, useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtClock, fmtHash } from "../../../lib/format";
import type { AuditResponse, OpsResponse } from "../../../lib/types";
import { Card, EmptyState, PageHeader, StatusChip } from "../../../components/ui";
import { DataTable, type Column } from "../../../components/Table";
import { Drawer } from "../../../components/Overlays";
import { Page } from "../../../components/AppShell";
import { Icon } from "../../../components/Icons";
import { SearchInput, Tabs } from "../../../components/patterns";
import { toneKind } from "./toneKind";
import { summarize } from "./summarize";

export default function Audit() {
  const [tab, setTab] = useState<"audit" | "ops">("ops");
  const [q, setQ] = useState("");
  const [page, setPage] = useState(0);
  const limit = 20;
  const audit = usePolled((s) => apiGet<AuditResponse>(`/audit?limit=200`, { signal: s }), 12000);
  const ops = usePolled((s) => apiGet<OpsResponse>(`/ops?limit=200`, { signal: s }), 8000);
  const [open, setOpen] = useState<Record<string, unknown> | null>(null);

  const rows = useMemo(() => {
    const src =
      tab === "audit"
        ? (audit.data?.items ?? []).map((r) => ({ ts: r["ts"] ?? 0, kind: String(r["action"] ?? "entry"), detail: r }))
        : (ops.data?.items ?? []).map((r) => ({ ts: r.ts, kind: r.kind, detail: r.detail }));
    const needle = q.trim().toLowerCase();
    return needle ? src.filter((r) => JSON.stringify(r).toLowerCase().includes(needle)) : src;
  }, [tab, audit.data, ops.data, q]);

  const cols: Column<(typeof rows)[number]>[] = [
    { key: "ts", label: "Time", sortValue: (r) => r.ts, render: (r) => <span className="t-code" style={{ color: "var(--text-3)" }}>{fmtClock(r.ts)}</span> },
    { key: "kind", label: "Event", render: (r) => <StatusChip dot={false} status={toneKind(r.kind)} label={r.kind.replaceAll("_", " ")} /> },
    { key: "corr", label: "Correlation", hideable: true, render: (r) => (r.detail["corr"] ? <code className="t-code">{fmtHash(String(r.detail["corr"]), 10)}</code> : "—") },
    { key: "resource", label: "Resource", render: (r) => summarize(r.detail) },
    { key: "result", label: "Result", render: (r) => (r.detail["result"] ? String(r.detail["result"]) : r.detail["status"] ? String(r.detail["status"]) : "—") },
  ];

  return (
    <Page>
      <PageHeader
        title="Audit Explorer"
        sub={tab === "audit" ? "HMAC-sealed journal entries (never rewritten)" : "operator notes + lifecycle feed, newest first"}
        actions={
          <Tabs
            ariaLabel="feed source"
            value={tab}
            onChange={(v) => { setTab(v); setPage(0); }}
            options={[
              { value: "ops", label: "Ops feed" },
              { value: "audit", label: "Audit journal" },
            ]}
          />
        }
      />

      {tab === "audit" && audit.data?.note && (
        <p className="t-caption" role="note"><Icon.Info size={12} /> {audit.data.note}</p>
      )}
      {tab === "audit" && rows.length === 0 && !audit.error && (
        <Card>
          <EmptyState
            icon={<Icon.Audit size={18} />}
            title="Audit journal is empty"
            body={
              <>
                Set <code className="t-code">audit_path</code> to start a sealed journal (and pair it with
                <code className="t-code"> attest_key_path</code> for HMAC evidence). Entries here are the
                same lines <code className="t-code">harvest audit</code> renders.
              </>
            }
          />
        </Card>
      )}

      {rows.length > 0 && (
        <Card>
          <div className="filterbar" style={{ justifyContent: "flex-end" }}>
            <span style={{ marginRight: "auto" }}>
              <SearchInput
                label="Filter loaded audit entries"
                placeholder="Filter loaded entries…"
                value={q}
                onChange={(v) => { setQ(v); setPage(0); }}
              />
            </span>
            <span className="t-caption">{rows.length} in view (window cap 200)</span>
          </div>
          <DataTable
            caption={tab === "audit" ? "audit journal" : "operator feed"}
            columns={cols}
            rows={rows.slice(page * limit, (page + 1) * limit)}
            rowKey={(r) => `${r.ts}:${r.kind}:${JSON.stringify(r.detail).slice(0, 48)}`}
            total={rows.length}
            limit={limit}
            offset={page * limit}
            onPage={(off) => setPage(Math.floor(off / limit))}
            loading={(tab === "audit" ? audit.state : ops.state) === "connecting"}
            error={tab === "audit" ? audit.error : ops.error}
            onRowClick={(r) => setOpen({ ts: r.ts, kind: r.kind, ...("detail" in r ? (r.detail as object) : {}) })}
            emptyTitle="No entries match"
            emptyBody="Try clearing the filter, or run some operations."
          />
        </Card>
      )}

      {open && (
        <Drawer
          title="Entry detail"
          onClose={() => setOpen(null)}
        >
          <p className="t-caption">Raw server payload (already redacted at the source; nothing here is re-hidden by the UI).</p>
          <pre className="code">{JSON.stringify(open, null, 2)}</pre>
        </Drawer>
      )}
    </Page>
  );
}
