// Diagnostics — split from Diagnostics.tsx; barrel re-exports keep every public path & name

import { useMemo, useState } from "react";
import { apiGet } from "../../../lib/api";
import { usePolled } from "../../../hooks/usePolled";
import { fmtDateTime, fmtMs, fmtNum } from "../../../lib/format";
import { maskSecrets } from "../../../lib/format";
import type { DiagnosticsResponse } from "../../../lib/types";
import { Button, Card, CopyButton, PageHeader, StatusChip, Segmented } from "../../../components/ui";
import { Page } from "../../../components/AppShell";
import { useConnection } from "../../../lib/connection";
import { useToasts } from "../../../hooks/useToasts";
import { Icon } from "../../../components/Icons";
import { formatVal } from "./formatVal";

export default function Diagnostics() {
  const diag = usePolled((s) => apiGet<DiagnosticsResponse>("/diagnostics", { signal: s }), 10000);
  const conn = useConnection();
  const { push } = useToasts();
  const [open, setOpen] = useState<Set<string>>(new Set());

  const d = diag.data;
  const report = useMemo(() => {
    if (!d) return "";
    const lines = [
      `# harvest diagnostics — ${new Date().toISOString()}`,
      `overall: ${d.overall} · generated ${fmtDateTime(d.generated_ts)}`,
      `api: ${conn.base || "same-origin"} · rtt ${conn.latencyMs ?? "—"} ms · cadence ${conn.intervalSecs}s`,
      `engine: ${conn.health?.running ? "running" : "idle"}`,
      "",
      ...d.components.map((c) =>
        `[${c.status.toUpperCase().padEnd(9)}] ${c.name}: ${
          Object.entries(c.detail).map(([k, v]) => `${k}=${v}`).join(", ") || "—"
        }`,
      ),
      "",
      `config validation: ${conn.config?.validation ?? "unavailable"}`,
      `warnings: ${(conn.config?._warnings ?? []).length}`,
    ];
    return maskSecrets(lines.join("\n"));
  }, [d, conn]);

  if (!d) {
    return (
      <Page>
        <PageHeader title="Diagnostics" sub={diag.error ? "diagnostics unreachable" : "loading…"} />
        <Card>
          <p className="t-caption">
            {diag.error ? `The admin API did not answer diagnostics (${diag.error.message}).` : "One moment."}
            {" "}
            <Button onClick={diag.refresh}><Icon.Refresh size={12} /> Retry</Button>
          </p>
        </Card>
      </Page>
    );
  }

  return (
    <Page>
      <PageHeader
        title="Diagnostics"
        sub={`component verdicts · generated ${fmtDateTime(d.generated_ts)}`}
        actions={
          <>
            <StatusChip status={d.overall === "unhealthy" ? "error" : d.overall} label={d.overall} />
            <CopyButton
              text={report}
              label="Copy diagnostic report"
              onCopied={() => push("ok", "Redacted diagnostic report copied")}
            />
            <Button variant="ghost" onClick={() => setOpen(new Set(d.components.map((c) => c.name)))}>Expand all</Button>
            <Button variant="ghost" onClick={() => setOpen(new Set())}>Collapse all</Button>
          </>
        }
      />

      <div style={{ display: "grid", gap: 8 }} role="list" aria-label="diagnostic components">
        {d.components.map((c) => {
          const isOpen = open.has(c.name);
          const tone = c.status === "healthy" ? "ok" : c.status === "disabled" ? "idle" : "err";
          return (
            <section key={c.name} className="card" role="listitem" aria-label={c.name}>
              <header
                className="card-head"
                style={{ cursor: "pointer" }}
                onClick={() =>
                  setOpen((s) => {
                    const n = new Set(s);
                    if (n.has(c.name)) n.delete(c.name);
                    else n.add(c.name);
                    return n;
                  })
                }
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    (e.currentTarget as HTMLElement).click();
                  }
                }}
                role="button"
                tabIndex={0}
                aria-expanded={isOpen}
              >
                <i className={`dot ${tone}`} aria-hidden="true" />
                <h2 className="t-section" style={{ textTransform: "capitalize" }}>{c.name.replaceAll("_", " ")}</h2>
                <span className="spacer" style={{ marginLeft: "auto" }} />
                <span className="t-caption">{Object.keys(c.detail).length} field(s)</span>
                <StatusChip status={c.status === "unhealthy" ? "error" : c.status} label={c.status} />
                <Icon.ChevronRight size={13} style={{ rotate: isOpen ? "90deg" : "0deg", transition: "rotate var(--t-fast)" }} />
              </header>
              {isOpen && (
                <div className="card-body">
                  <dl className="kv" style={{ margin: 0 }}>
                    {Object.entries(c.detail).map(([k, v]) => (
                      <span key={k} style={{ display: "contents" }}>
                        <dt>{k.replaceAll("_", " ")}</dt>
                        <dd>{formatVal(k, v)}</dd>
                      </span>
                    ))}
                  </dl>
                </div>
              )}
            </section>
          );
        })}
      </div>

      <Card title="Console-side facts (measured, not the server's)">
        <dl className="kv" style={{ margin: 0 }}>
          <dt>API round-trip</dt><dd>{conn.latencyMs !== null ? fmtMs(conn.latencyMs) : "—"}</dd>
          <dt>Fetch cadence</dt>
          <dd style={{ display: "inline-flex", gap: 10, alignItems: "center", flexWrap: "wrap" }}>
            <Segmented
              ariaLabel="Shell fetch cadence"
              value={String(conn.intervalSecs)}
              onChange={(v) => conn.setIntervalSecs(Number(v))}
              options={[
                { value: "3", label: "3 s" },
                { value: "6", label: "6 s" },
                { value: "15", label: "15 s" },
                { value: "30", label: "30 s" },
              ]}
            />
            <span className="t-caption">shell-wide · per-page intervals are noted in their headers</span>
          </dd>
          <dt>Live state</dt><dd>{conn.state}</dd>
          <dt>Config warnings</dt><dd>{conn.config ? fmtNum(conn.config._warnings.length) : "—"}</dd>
          <dt>Production hardening notes</dt><dd>{conn.config ? (conn.config._production.length ? conn.config._production.join(" · ") : "none") : "—"}</dd>
        </dl>
        <p className="t-caption">
          The server deliberately runs no extra probes from /diagnostics (no new connections on a
          scrape loop) — DB/redis live checks exist as CLI: <code className="t-code">harvest health</code>.
        </p>
      </Card>
    </Page>
  );
}
