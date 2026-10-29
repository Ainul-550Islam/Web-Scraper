// Usage — the managed API's metering view.
//
// One read of `GET /v1/admin/usage`, folded into metric cards, a per-key table
// and a per-day series. The export buttons post to `POST /v1/admin/usage/export`
// with the same filter as query params, and show the manifest row the server
// returns — the file itself is written server-side, in the operator's export
// directory, not in the browser.

import { useState } from "react";
import { Button, Card, EmptyState, Field, MetricCard, NumberInput, PageHeader, SelectInput, Skeleton, StatusChip } from "../../../components/ui";
import { Icon } from "../../../components/Icons";
import { Page } from "../../../components/AppShell";
import { usePolled } from "../../../hooks/usePolled";
import { useToasts } from "../../../hooks/useToasts";
import { fmtBytes, fmtNum, fmtDateTime } from "../../../lib/format";
import { exportUsage, fetchUsage, type UsageWindow } from "../keys/KeysClient";
import type { UsageExport } from "../keys/types";

/** The row cap the server applies to a single report; matches V1_JOBS_MAX_LIST. */
const DEFAULT_LIMIT = 200;
/** The ledger is append-only and cheap to re-read; refresh on this cadence. */
const POLL_MS = 60_000;

function fromDateInput(value: string): number | undefined {
  if (!value) return undefined;
  const t = Date.parse(`${value}T00:00:00Z`);
  return Number.isFinite(t) ? t : undefined;
}

export function Usage() {
  // `draft` is what the form holds; `applied` is what the server is asked for.
  // The report is polled against `applied`, so changing a field does not fetch
  // until Apply or Reset commits it.
  const [draft, setDraft] = useState<{
    keyId: string;
    since: string;
    until: string;
    limit: number | null;
  }>({ keyId: "", since: "", until: "", limit: DEFAULT_LIMIT });
  const [applied, setApplied] = useState<UsageWindow>({
    keyId: undefined,
    sinceMs: undefined,
    untilMs: undefined,
    limit: DEFAULT_LIMIT,
  });
  const [exported, setExported] = useState<UsageExport | null>(null);
  const toasts = useToasts();

  const poll = usePolled(
    (signal) => fetchUsage({ ...applied }, signal),
    POLL_MS,
    JSON.stringify(applied),
  );
  const report = poll.data;
  const loading = poll.state === "refreshing" || poll.state === "connecting";
  const error = poll.error?.message ?? null;

  const apply = () =>
    setApplied({
      keyId: draft.keyId || undefined,
      sinceMs: fromDateInput(draft.since),
      untilMs: fromDateInput(draft.until),
      limit: draft.limit ?? undefined,
    });

  const reset = () => {
    setDraft({ keyId: "", since: "", until: "", limit: DEFAULT_LIMIT });
    setApplied({ keyId: undefined, sinceMs: undefined, untilMs: undefined, limit: DEFAULT_LIMIT });
  };

  async function doExport(format: "jsonl" | "csv") {
    try {
      const out = await exportUsage({ ...applied, format });
      setExported(out);
      toasts.push("ok", `Exported ${out.rows} row(s) to ${out.path}`);
    } catch (e) {
      toasts.push("err", e instanceof Error ? e.message : "could not export");
    }
  }

  const totals = report?.totals;
  const keyId = applied.keyId ?? "";
  const keyOptions = report?.by_key.map((r) => r.key_id) ?? [];
  const scoped = report?.by_key.filter((r) => !keyId || r.key_id === keyId) ?? [];

  return (
    <Page>
      <PageHeader
        title="Usage"
        sub={
          report
            ? `${fmtNum(report.key_count)} key(s) · ${fmtNum(totals?.requests ?? 0)} requests · ${fmtNum(
                totals?.units ?? 0,
              )} units`
            : "reading the ledger"
        }
        actions={
          report ? (
            <StatusChip
              status={totals?.errors ? "degraded" : "ok"}
              label={`ledger head seq ${report.ledger_head?.seq ?? "—"}`}
            />
          ) : (
            <StatusChip status="idle" label="—" />
          )
        }
      />

      <Card title="Filter">
        <div className="grid grid-3">
          <Field label="Key">
            <SelectInput
              aria-label="Key"
              value={draft.keyId}
              onChange={(e) => setDraft((d) => ({ ...d, keyId: e.target.value }))}
            >
              <option value="">All keys</option>
              {keyOptions.map((id) => (
                <option key={id} value={id}>
                  {id}
                </option>
              ))}
            </SelectInput>
          </Field>
          <Field label="Since (UTC day)" help="Inclusive lower bound.">
            <input
              type="date"
              className="input"
              value={draft.since}
              onChange={(e) => setDraft((d) => ({ ...d, since: e.target.value }))}
            />
          </Field>
          <Field label="Until (UTC day)" help="Inclusive upper bound.">
            <input
              type="date"
              className="input"
              value={draft.until}
              onChange={(e) => setDraft((d) => ({ ...d, until: e.target.value }))}
            />
          </Field>
          <Field label="Row limit" help="Caps the per-key and per-day rows returned.">
            <NumberInput
              value={draft.limit}
              onValue={(v) => setDraft((d) => ({ ...d, limit: v }))}
              min={1}
              step={100}
            />
          </Field>
        </div>
        <div className="row">
          <Button variant="primary" onClick={apply} disabled={loading}>
            {loading ? "Reading…" : "Apply filter"}
          </Button>
          <Button
            variant="ghost"
            onClick={reset}
          >
            Reset
          </Button>
          <span className="t-caption">
            since {draft.since || "—"} until {draft.until || "—"}
          </span>
        </div>
      </Card>

      {error ? (
        <div className="state err" role="alert">
          <div className="state-ico"><Icon.Warn size={20} /></div>
          <h3>Could not read the usage report</h3>
          <p>{error}</p>
          <Button onClick={() => poll.refresh()}><Icon.Refresh size={13} /> Retry</Button>
        </div>
      ) : loading && !report ? (
        <div style={{ display: "grid", gap: 8 }}>
          <Skeleton />
          <Skeleton h={10} />
          <Skeleton />
          <Skeleton h={10} />
        </div>
      ) : report && totals ? (
        <>
          <div className="grid grid-metrics" role="list" aria-label="Usage totals">
            <MetricCard label="Requests" value={fmtNum(totals.requests)} />
            <MetricCard label="Units billed" value={fmtNum(totals.units)} />
            <MetricCard label="Bytes delivered" value={fmtBytes(totals.bytes)} />
            <MetricCard label="Succeeded" value={fmtNum(totals.ok)} tone="ok" />
            <MetricCard label="Refused" value={fmtNum(totals.refused)} tone="warn" />
            <MetricCard
              label="Errors"
              value={fmtNum(totals.errors)}
              tone={totals.errors ? "err" : "ok"}
            />
          </div>

          <Card title="Per key" actions={<span className="t-caption">{scoped.length} row(s)</span>}>
            {scoped.length === 0 ? (
              <EmptyState icon={<Icon.Usage size={16} />} title="No usage in this window" body="Adjust the filter, or issue a key and make a request." />
            ) : (
              <table className="data">
                <thead>
                  <tr>
                    <th scope="col">Key</th>
                    <th scope="col" className="num">Requests</th>
                    <th scope="col" className="num">Units</th>
                    <th scope="col" className="num">Bytes</th>
                    <th scope="col" className="num">Ok / refused / errors</th>
                    <th scope="col">First / last</th>
                  </tr>
                </thead>
                <tbody>
                  {scoped.map((r) => (
                    <tr key={r.key_id}>
                      <td>
                        <code className="t-code">{r.key_id}</code>
                        <div className="t-caption">{r.tenant_id}</div>
                      </td>
                      <td className="num">{fmtNum(r.requests)}</td>
                      <td className="num">{fmtNum(r.units)}</td>
                      <td className="num">{fmtBytes(r.bytes)}</td>
                      <td className="num t-caption">
                        {fmtNum(r.ok)} / {fmtNum(r.refused)} / {fmtNum(r.errors)}
                      </td>
                      <td className="t-caption">
                        {fmtDateTime(r.first_ts_ms)} → {fmtDateTime(r.last_ts_ms)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>

          <Card title="Per day" actions={<span className="t-caption">{report.by_day.length} day(s)</span>}>
            {report.by_day.length === 0 ? (
              <EmptyState title="No days in this window" />
            ) : (
              <table className="data">
                <thead>
                  <tr>
                    <th scope="col">UTC day</th>
                    <th scope="col" className="num">Requests</th>
                    <th scope="col" className="num">Units</th>
                    <th scope="col" className="num">Bytes</th>
                  </tr>
                </thead>
                <tbody>
                  {report.by_day.map((d) => (
                    <tr key={d.day}>
                      <td>{d.day}</td>
                      <td className="num">{fmtNum(d.requests)}</td>
                      <td className="num">{fmtNum(d.units)}</td>
                      <td className="num">{fmtBytes(d.bytes)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Card>

          <Card title="Export">
            <p className="t-caption">
              Writes into the operator's <code className="t-code">managed_api.export_dir</code> with a
              manifest recording rows, bytes and the payload's SHA-256. Rows are capped; the manifest
              says so when the cap bit.
            </p>
            <div className="row">
              <Button onClick={() => void doExport("jsonl")}>Export JSONL</Button>
              <Button onClick={() => void doExport("csv")}>Export CSV</Button>
            </div>
            {exported && (
              <dl className="kv" style={{ marginTop: 12 }}>
                <dt>File</dt>
                <dd><code className="t-code">{exported.path}</code></dd>
                <dt>Manifest</dt>
                <dd><code className="t-code">{exported.manifest}</code></dd>
                <dt>Rows</dt>
                <dd>
                  {fmtNum(exported.rows)} · {fmtBytes(exported.bytes)} · sha256{" "}
                  <code className="t-code">{exported.sha256.slice(0, 16)}…</code>
                  {exported.truncated ? " · truncated" : ""}
                </dd>
              </dl>
            )}
          </Card>
        </>
      ) : null}
    </Page>
  );
}

export default Usage;
