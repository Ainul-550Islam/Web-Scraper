// ConfigStudio — split from ConfigStudio.tsx; barrel re-exports keep every public path & name

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useConnection } from "../../../lib/connection";
import { CONTROLS, CONTROLS_BY_SECTION, crossFieldErrors, getPath, setPath, validateControl } from "../../../lib/configSchema";
import type { ConfigDocument } from "../../../lib/types";
import { Button, Card, CopyButton, PageHeader, StatusChip, Segmented } from "../../../components/ui";
import { useToasts } from "../../../hooks/useToasts";
import { Page } from "../../../components/AppShell";
import { FormSection } from "../../../components/patterns";
import { Icon } from "../../../components/Icons";
import { TargetBuilder } from "../TargetBuilder";
import { SENSITIVE } from "./SENSITIVE";
import { SENSITIVE_HINT } from "./SENSITIVE_HINT";
import { Control } from "./Control";
import { StudioIndex } from "./StudioIndex";


/**
 * The console is a DRAFT editor: the admin API is read-only for config
 * (validate is enforced server-side at startup — `harvest validate`).
 * Values shown/edited come from `GET /config` verbatim; secret fields are
 * server-redacted and remain non-editable. Downloads produce a config file
 * to hand back to the CLI — nothing is POSTed.
 */
export default function ConfigStudio() {
  const conn = useConnection();
  const { push } = useToasts();
  const server = conn.config;
  const [draft, setDraft] = useState<ConfigDocument | null>(null);
  const [jsonView, setJsonView] = useState<"form" | "json">("form");
  const [builderTarget, setBuilderTarget] = useState<number | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  // seed draft from server doc; reseed when a different server doc arrives
  useEffect(() => {
    if (server) setDraft(structuredClone(server));
  }, [server?.config_sha256]); // eslint-disable-line react-hooks/exhaustive-deps

  // ?target= from the command palette
  useEffect(() => {
    const t = new URLSearchParams(window.location.hash.split("?")[1] ?? "").get("target");
    if (t && draft) {
      const idx = draft.targets.findIndex((x) => x.id === t);
      if (idx >= 0) setBuilderTarget(idx);
    }
  }, [draft]);

  const dirtyPaths = useMemo(() => {
    if (!draft || !server) return new Set<string>();
    const diff = new Set<string>();
    for (const c of CONTROLS) {
      const a = JSON.stringify(getPath(draft, c.path));
      const b = JSON.stringify(getPath(server, c.path));
      if (a !== b) diff.add(c.path);
    }
    if (JSON.stringify(draft.targets) !== JSON.stringify(server.targets)) diff.add("targets");
    return diff;
  }, [draft, server]);

  const fieldErrors = useMemo(() => {
    const errs = new Map<string, string>();
    if (!draft) return errs;
    for (const c of CONTROLS) {
      const e = validateControl(c, getPath(draft, c.path));
      if (e) errs.set(c.path, e);
    }
    for (const e of crossFieldErrors(draft)) errs.set("__cross__", (errs.get("__cross__") ?? "") + e + ";\n");
    return errs;
  }, [draft]);

  const exportable = useCallback((): ConfigDocument | null => {
    if (!draft) return null;
    const d = structuredClone(draft) as unknown as Record<string, unknown>;
    // strip server-provided extras — they are not config fields
    for (const k of Object.keys(d)) {
      if (k.startsWith("_") || k === "config_sha256" || k === "validation" || k === "load_warnings") delete d[k];
    }
    return d as unknown as ConfigDocument;
  }, [draft]);

  if (!conn.config) {
    return (
      <Page>
        <PageHeader title="Configuration Studio" sub="needs GET /config from the running engine" />
        <Card>
          <p className="t-caption">
            Start <code className="t-code">harvest serve</code> (or attach via the same origin) — the studio reads the live
            document rather than a static file, so what you see is what the engine validated.
          </p>
        </Card>
      </Page>
    );
  }

  return (
    <Page>
      <PageHeader
        title="Configuration Studio"
        sub={
          <>
            server document sha {server?.config_sha256.slice(0, 12)}… · {CONTROLS.length} mapped controls (each verified against src/config.rs) ·
            {" "}{dirtyPaths.size ? <strong style={{ color: "var(--warn)" }}>{dirtyPaths.size} changed in draft</strong> : "draft matches server"}
          </>
        }
        actions={
          <>
            <StatusChip
              status={fieldErrors.size ? "error" : server?.validation === "ok" ? "ok" : "degraded"}
              label={fieldErrors.size ? `${fieldErrors.size} local error(s)` : `server: ${server?.validation}`}
            />
            <Segmented
              ariaLabel="view mode"
              value={jsonView}
              onChange={setJsonView}
              options={[{ value: "form", label: "Form" }, { value: "json", label: "JSON" }]}
            />
          </>
        }
      />

      {server && server.validation !== "ok" && (
        <Card>
          <div className="field-error" role="alert" style={{ whiteSpace: "pre-line" }}>
            <Icon.Warn size={13} /> server validation reported: {String(server.validation)}
          </div>
        </Card>
      )}
      <StudioIndex dirtyPaths={dirtyPaths} fieldErrors={fieldErrors} />

      {server && server._warnings.length > 0 && (
        <p className="t-caption" role="note">
          <Icon.Info size={12} /> server warnings: {server._warnings.join(" · ")}
        </p>
      )}
      {server && server._production.length > 0 && (
        <Card>
          <p style={{ margin: 0, color: "var(--warn)", fontSize: 13 }}>
            <Icon.Warn size={13} /> Production hardening checklist (server-computed):
            {" "}{server._production.join(" · ")}
          </p>
        </Card>
      )}

      <Card
        title="Targets"
        actions={<span className="t-caption">{draft?.targets.length ?? 0} target(s)</span>}
      >
        <div className="grid grid-3">
          {(draft?.targets ?? []).map((t, i) => (
            <button key={t.id} type="button" className="pipe-node" style={{ textAlign: "left" }} onClick={() => setBuilderTarget(i)}>
              <span className="pn-title"><Icon.Jobs size={12} /> {t.id} {dirtyPaths.has("targets") && <span className="chip warn">draft</span>}</span>
              <span className="pn-meta">{t.mode} · depth {t.max_depth} · {Object.keys(t.extract).length} fields</span>
              <span className="t-caption" style={{ display: "block", marginTop: 3, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: 230 }}>
                {t.start_url}
              </span>
            </button>
          ))}
        </div>
      </Card>

      {jsonView === "json" ? (
        <Card
          title="Draft JSON"
          actions={
            <>
              <CopyButton text={JSON.stringify(exportable(), null, 2)} onCopied={() => push("ok", "Draft JSON copied")} />
              <Button onClick={() => {
                const blob = new Blob([JSON.stringify(exportable(), null, 2) + "\n"], { type: "application/json" });
                const a = document.createElement("a");
                a.href = URL.createObjectURL(blob);
                a.download = "harvest-config.draft.json";
                a.click();
                URL.revokeObjectURL(a.href);
                push("ok", "Draft downloaded — validate with: harvest validate --config harvest-config.draft.json");
              }}><Icon.Download size={12} /> Download</Button>
            </>
          }
        >
          <pre className="code" tabIndex={0} aria-label="draft configuration JSON">{JSON.stringify(exportable(), null, 2)}</pre>
        </Card>
      ) : (
        draft && (
          <div style={{ display: "grid", gap: "var(--s-4)" }}>
            {fieldErrors.has("__cross__") && (
              <Card>
                <div className="field-error" role="alert" style={{ whiteSpace: "pre-line" }}>
                  <Icon.Warn size={13} /> {fieldErrors.get("__cross__")}
                </div>
              </Card>
            )}
            {CONTROLS_BY_SECTION.map(({ section, controls }) => (
              <FormSection
                key={section}
                id={section.toLowerCase()}
                title={section}
                count={controls.length}
                badge={controls.some((c) => dirtyPaths.has(c.path)) ? <span className="chip warn">draft changes</span> : undefined}
                warning={
                  controls.some((c) => SENSITIVE.has(c.path) && dirtyPaths.has(c.path)) ? (
                    <>this section changed a sensitive control — {SENSITIVE_HINT}</>
                  ) : controls.some((c) => SENSITIVE.has(c.path)) ? <>{SENSITIVE_HINT}</> : undefined
                }
              >
                <div className="ctrl-grid">
                    {controls.map((c) =>
                      draft ? (
                        <Control
                          key={c.path}
                          def={c}
                          draft={draft}
                          server={server!}
                          dirty={dirtyPaths.has(c.path)}
                          error={fieldErrors.get(c.path) ?? null}
                          onChange={(v) => setDraft((d) => (d ? setPath(d, c.path, v) : d))}
                          onReset={() => {
                            const v = getPath(server, c.path);
                            setDraft((d) => (d ? setPath(d, c.path, structuredClone(v)) : d));
                          }}
                        />
                      ) : null,
                    )}
                </div>
              </FormSection>
            ))}
            <Card
              title="Apply"
              foot={
                <span className="t-caption">
                  No config write endpoint exists — and that is deliberate. Download the draft, run{" "}
                  <code className="t-code">harvest validate --config …</code>, then restart/serve with it.
                </span>
              }
            >
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                <Button variant="primary" disabled={!dirtyPaths.size} onClick={() => {
                  const blob = new Blob([JSON.stringify(exportable(), null, 2) + "\n"], { type: "application/json" });
                  const a = document.createElement("a");
                  a.href = URL.createObjectURL(blob);
                  a.download = "harvest-config.draft.json";
                  a.click();
                  URL.revokeObjectURL(a.href);
                  push("ok", "Draft exported. Validate: harvest validate --config harvest-config.draft.json");
                }}>
                  <Icon.Download size={13} /> Export draft ({dirtyPaths.size})
                </Button>
                <Button disabled={!dirtyPaths.size} onClick={() => { setDraft(structuredClone(server!)); push("info", "Draft discarded"); }}>
                  Discard all
                </Button>
                <span style={{ marginLeft: "auto" }}>
                  <Button variant="ghost" onClick={() => fileRef.current?.click()}>Load a config file…</Button>
                  <input ref={fileRef} type="file" accept="application/json,.json" hidden aria-label="load config file" onChange={async (e) => {
                    const f = e.target.files?.[0];
                    if (!f) return;
                    try {
                      const parsed = JSON.parse(await f.text()) as ConfigDocument;
                      if (typeof parsed !== "object" || parsed === null || !("targets" in parsed)) throw new Error("missing targets");
                      setDraft(parsed);
                      push("ok", `Loaded ${f.name} into the draft (local only)`);
                    } catch (err) {
                      push("err", `Could not parse ${f.name}: ${err instanceof Error ? err.message : "invalid JSON"}`);
                    }
                    e.target.value = "";
                  }} />
                </span>
              </div>
            </Card>
          </div>
        )
      )}

      {builderTarget !== null && draft && (
        <TargetBuilder
          target={draft.targets[builderTarget]!}
          onChange={(t) =>
            setDraft((d) => {
              if (!d) return d;
              const targets = [...d.targets];
              targets[builderTarget] = t;
              return { ...d, targets };
            })
          }
          onClose={() => setBuilderTarget(null)}
        />
      )}
    </Page>
  );
}
