/**
 * Run Detail — deep-dive for one entry of the run ledger.
 *
 * Address: `/run?run=<config name>` (optionally `&ts=<unix secs>` to pin a
 * specific entry when a name has many runs). Data source: `GET /runs?limit=64`
 * — the same bounded newest-window the Runs table reads — enriched with the
 * matching `GET /recent` row (which is the only endpoint that embeds the full
 * summary block). Everything else stays honestly empty: no metrics are
 * invented for runs the server does not describe.
 */
import { useMemo } from "react";
import { apiGet } from "../../lib/api";
import { usePolled } from "../../hooks/usePolled";
import { useRoute } from "../../app/router";
import type { RecentRunRow, RunsResponse } from "../../lib/types";
import { fmtDateTime, fmtHash, fmtMs, fmtNum } from "../../lib/format";
import { epochToIso, humanDuration, statusLabel } from "../../lib/dataFormatters";
import { Page } from "../../components/AppShell";
import { Button, Card, EmptyState, MetricCard, PageHeader, StatusChip } from "../../components/ui";

export default function RunDetail() {
  const route = useRoute();
  const wantRun = route.query.get("run") ?? "";
  const wantTs = Number(route.query.get("ts") ?? "");

  const runs = usePolled(
    (s) => apiGet<RunsResponse>("/runs?limit=64&order=desc&sort=ts", { signal: s }),
    15000,
  );
  const recent = usePolled((s) => apiGet<RecentRunRow[]>("/recent", { signal: s }), 15000);

  const match = useMemo(() => {
    const items = runs.data?.page.items ?? [];
    const byName = wantTs
      ? items.filter((r) => r.run === wantRun && r.ts === wantTs)
      : items.filter((r) => r.run === wantRun);
    return byName.length > 0 ? byName[0] : null;
  }, [runs.data, wantRun, wantTs]);

  const detail: RecentRunRow | null = useMemo(() => {
    if (!match) return null;
    const rows = (recent.data ?? []).filter((r) => r.run === match.run && r.ts === match.ts);
    // ledger `index` is the disambiguator; /recent rows may omit it, so
    // prefer an exact triple and fall back to the single run+ts row
    return rows.find((r) => r.index === match.index) ?? rows[0] ?? null;
  }, [recent.data, match]);

  if (runs.error) {
    return (
      <Page>
        <PageHeader title="Run Detail" sub="history endpoint unreachable" />
        <EmptyState title="Run history did not answer" body={`GET /runs failed (${runs.error.message}). Start the engine or check the token.`} />
      </Page>
    );
  }
  if (!match) {
    return (
      <Page>
        <PageHeader title="Run Detail" sub={wantRun ? `no entry for “${wantRun}” in the fetched window` : "no run selected"} />
        <EmptyState
          title={wantRun ? "Run not found" : "Select a run to inspect"}
          body={
            wantRun
              ? "The newest 64 history entries do not include that name (older than the window, or the config was renamed)."
              : "Open this page from the Runs table, or pass /run?run=<config name>."
          }
          action={
            <Button variant="ghost" onClick={() => (window.location.hash = "/runs")}>
              Browse all runs
            </Button>
          }
        />
      </Page>
    );
  }

  const s = detail?.summary;
  const perTarget = s ? Object.entries(s.per_target) : [];
  const num = (v: unknown): number => (typeof v === "number" && Number.isFinite(v) ? v : 0);
  return (
    <Page>
      <PageHeader
        title={`Run — ${match.run}`}
        sub={`${epochToIso(match.ts)} · ${fmtDateTime(match.ts)} engine-local · config ${fmtHash(match.config_sha256)}`}
        actions={<StatusChip status={match.status} label={statusLabel(match.status)} />}
      />

      <div className="grid grid-metrics">
        <MetricCard label="Duration" value={humanDuration(match.duration_ms) ?? fmtMs(match.duration_ms)} meta={`${fmtMs(match.duration_ms)} exact`} />
        <MetricCard label="Written" value={fmtNum(match.written)} meta={s ? `${fmtNum(s.pages_fetched)} pages fetched` : undefined} />
        <MetricCard label="Quarantined" value={fmtNum(match.quarantined)} tone={match.quarantined > 0 ? "warn" : "ok"} />
        <MetricCard label="Error" value={match.error ? "recorded" : "none"} tone={match.error ? "err" : undefined} meta={match.error ?? undefined} />
      </div>

      <Card title="Run record" hover>
        <dl className="kv">
          <dt>ts</dt><dd>{match.ts} — {epochToIso(match.ts)}</dd>
          <dt>index</dt><dd>{match.index}</dd>
          <dt>status</dt><dd>{statusLabel(match.status)}</dd>
          <dt>config sha256</dt><dd title={match.config_sha256}>{match.config_sha256}</dd>
          {detail && s && (
            <>
              <dt>respect_robots</dt><dd>{String(detail.respect_robots)}</dd>
              <dt>requests</dt><dd>{fmtNum(num(s.requests_total))}</dd>
              <dt>retries</dt><dd>{fmtNum(num(s.retries))}</dd>
              <dt>deduped</dt><dd>{fmtNum(num(s.deduped))}</dd>
              <dt>blocked</dt><dd>{fmtNum(num(s.blocked))}</dd>
              <dt>robots denied</dt><dd>{fmtNum(num(s.robots_denied))}</dd>
              <dt>checkpoints</dt><dd>{fmtNum(num(s.ckpt_writes))}</dd>
              <dt>revalidated</dt><dd>{fmtNum(num(s.revalidated))}</dd>
            </>
          )}
        </dl>
        {!detail && (
          <p className="t-caption t-muted">
            /recent no longer carries this row's full summary (it keeps only the newest entries) —
            ledger fields above are still authoritative.
          </p>
        )}
      </Card>

      {s && (
        <Card title="Per-target tallies" hover>
          {perTarget.length === 0 ? (
            <p className="t-caption t-muted">The summary embedded no per-target block.</p>
          ) : (
            <table className="data">
              <thead>
                <tr><th>target</th><th>written</th><th>quarantined</th><th>failed</th><th>blocked</th><th>revalidated</th></tr>
              </thead>
              <tbody>
                {perTarget.map(([t, v]) => (
                  <tr key={t}>
                    <td>{t}</td>
                    <td>{fmtNum(v.written)}</td>
                    <td>{fmtNum(v.quarantined)}</td>
                    <td>{v.failed > 0 ? <span className="t-err">{fmtNum(v.failed)}</span> : "0"}</td>
                    <td>{fmtNum(v.blocked)}</td>
                    <td>{fmtNum(v.revalidated)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Card>
      )}
    </Page>
  );
}
