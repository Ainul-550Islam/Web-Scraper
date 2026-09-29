// CommandComposer — the one form that produces a control-plane command body.
//
// It is deliberately boring: every field maps to a field of
// `harvest::control::CommandSpec`, every ceiling is the server's constant, and
// the validation below is a *convenience mirror* of
// `CommandLimits::validate` — the server re-validates every value, so a bug
// here can only waste a round trip, never widen what the engine will do.
//
// None of these fields are secrets: the composer never asks for a token, a
// provider key or a filesystem path, because the control plane does not accept
// any of them.

import { useMemo, useState } from "react";
import { Button, Card, CopyButton, Field, NumberInput, Segmented, SelectInput, TextInput, Toggle } from "../ui";
import { FormSection } from "./FormSection";

export const CONTROL_OPS = ["crawl", "export", "ai"] as const;
export type ControlOp = (typeof CONTROL_OPS)[number];

/* Ceilings mirrored from src/control.rs — keep in step, and say so in the UI. */
export const CEILINGS = {
  maxPages: 1_000_000, // crawl::MAX_PAGES_CEILING
  maxDepth: 20, // crawl::MAX_DEPTH_CEILING
  maxRecords: 5_000_000, // MAX_RECORDS_CEILING
  maxExportBytes: 1_073_741_824, // MAX_EXPORT_BYTES_CEILING (1 GiB)
  maxAiInputBytes: 4_194_304, // MAX_AI_INPUT_CEILING (4 MiB)
  timeoutSecs: 3_600, // MAX_TIMEOUT_SECS
  maxConcurrentJobs: 8, // MAX_CONCURRENT_JOBS
  maxQueueDepth: 64, // MAX_QUEUE_DEPTH
} as const;

export const EXPORT_FORMATS = ["csv", "jsonl", "xlsx", "pdf", "parquet", "xml", "zip"] as const;
export const AI_TASKS = ["extract", "classify", "normalize", "schema-suggest", "label"] as const;
/** The two providers this build registers; there is no way to ask for another. */
export const AI_PROVIDERS = ["deterministic-local", "disabled"] as const;

/** A compact, non-blocking note (the ui `InlineError` takes an ApiError). */
function Note({ message }: { message: string }) {
  return (
    <p className="field-error" role="status" style={{ margin: 0 }}>
      {message}
    </p>
  );
}

/** A labelled switch: `Toggle` renders the control only, so pair them here. */
function Switch({ label, checked, onChange }: { label: string; checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
      <Toggle label={label} checked={checked} onChange={onChange} />
      <span className="t-caption">{label}</span>
    </span>
  );
}

export interface CommandLimits {
  max_pages: number;
  max_depth: number;
  max_records: number;
  max_export_bytes: number;
  max_ai_input_bytes: number;
  timeout_secs: number;
}

export interface CrawlDraft {
  targets: string[];
  limits: CommandLimits;
}

export interface ExportDraft {
  formats: string[];
  dataset: string;
  bundle: boolean;
  verify: boolean;
  max_records: number;
  limits: CommandLimits;
}

export interface AiDraft {
  task: string;
  provider: string;
  confidence: number;
  external_allowed: boolean;
  source_kind: "run_output" | "inline";
  record_limit: number;
  field_limit: number;
  limits: CommandLimits;
}

export type CommandDraft =
  | { op: "crawl"; crawl: CrawlDraft }
  | { op: "export"; export: ExportDraft }
  | { op: "ai"; ai: AiDraft };

const LIMITS: CommandLimits = {
  max_pages: 100,
  max_depth: 1,
  max_records: 100_000,
  max_export_bytes: 268_435_456,
  max_ai_input_bytes: 1_048_576,
  timeout_secs: 900,
};

// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function emptyDraft(op: ControlOp, targetIds: string[] = []): CommandDraft {
  if (op === "crawl") {
    return {
      op: "crawl",
      crawl: {
        targets: targetIds.length === 1 ? [targetIds[0]!] : [],
        limits: { ...LIMITS },
      },
    };
  }
  if (op === "export") {
    return {
      op: "export",
      export: {
        formats: ["csv", "jsonl"],
        dataset: "",
        bundle: false,
        verify: true,
        max_records: 0,
        limits: { ...LIMITS },
      },
    };
  }
  return {
    op: "ai",
    ai: {
      task: "classify",
      provider: "deterministic-local",
      confidence: 0.9,
      external_allowed: false,
      source_kind: "run_output",
      record_limit: 100,
      field_limit: 32,
      limits: { ...LIMITS },
    },
  };
}

/** The JSON body the route expects — nothing more, nothing renamed. */
// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function buildCommand(draft: CommandDraft): Record<string, unknown> {
  if (draft.op === "crawl") {
    return {
      op: "crawl",
      targets: draft.crawl.targets,
      limits: draft.crawl.limits,
    };
  }
  if (draft.op === "export") {
    const e = draft.export;
    return {
      op: "export",
      formats: e.formats,
      dataset: e.dataset.trim() ? e.dataset.trim() : null,
      bundle: e.bundle,
      verify: e.verify,
      max_records: e.max_records > 0 ? e.max_records : null,
      limits: e.limits,
    };
  }
  const a = draft.ai;
  return {
    op: "ai",
    task: a.task,
    provider: a.provider,
    confidence: a.confidence,
    external_allowed: a.external_allowed,
    source:
      a.source_kind === "inline"
        ? { kind: "inline", fields: [] }
        : { kind: "run_output", record_limit: a.record_limit, field_limit: a.field_limit },
    limits: a.limits,
  };
}

export interface CommandIssue {
  field: string;
  message: string;
}

/**
 * Client-side mirror of the server's validation. Anything it catches, the
 * server would have caught too — this only saves the round trip and tells the
 * operator *before* they press submit.
 */
// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function validateCommand(draft: CommandDraft, targetIds: string[] = []): CommandIssue[] {
  const issues: CommandIssue[] = [];
  const limits =
    draft.op === "crawl"
      ? draft.crawl.limits
      : draft.op === "export"
        ? draft.export.limits
        : draft.ai.limits;
  const check = (field: string, value: number, ceiling: number, min = 1) => {
    if (!Number.isFinite(value) || value < min) {
      issues.push({ field, message: `${field} must be at least ${min}` });
    } else if (value > ceiling) {
      issues.push({ field, message: `${field} ${value} exceeds the ceiling ${ceiling}` });
    }
  };
  check("limits.max_pages", limits.max_pages, CEILINGS.maxPages);
  check("limits.max_depth", limits.max_depth, CEILINGS.maxDepth, 0);
  check("limits.max_records", limits.max_records, CEILINGS.maxRecords);
  check("limits.max_export_bytes", limits.max_export_bytes, CEILINGS.maxExportBytes);
  check("limits.max_ai_input_bytes", limits.max_ai_input_bytes, CEILINGS.maxAiInputBytes);
  check("limits.timeout_secs", limits.timeout_secs, CEILINGS.timeoutSecs);

  if (draft.op === "crawl") {
    if (draft.crawl.targets.length === 0) {
      issues.push({ field: "targets", message: "pick at least one target" });
    }
    for (const t of draft.crawl.targets) {
      if (targetIds.length > 0 && !targetIds.includes(t)) {
        issues.push({ field: "targets", message: `the attached config declares no target \`${t}\`` });
      }
    }
  }
  if (draft.op === "export" && draft.export.formats.length === 0) {
    issues.push({ field: "formats", message: "pick at least one format" });
  }
  if (draft.op === "ai") {
    if (draft.ai.confidence < 0 || draft.ai.confidence > 1) {
      issues.push({ field: "confidence", message: "confidence must be within [0, 1]" });
    }
    if (draft.ai.external_allowed && draft.ai.provider === "disabled") {
      issues.push({
        field: "external_allowed",
        message: "consent has no effect for the disabled provider",
      });
    }
    if (draft.ai.source_kind === "run_output") {
      check("source.record_limit", draft.ai.record_limit, 512);
      check("source.field_limit", draft.ai.field_limit, 512);
    }
  }
  return issues;
}

export function CommandComposer({
  draft,
  onChange,
  targetIds = [],
  onSubmit,
  submitting = false,
  error,
  idempotencyKey,
}: {
  draft: CommandDraft;
  onChange: (d: CommandDraft) => void;
  targetIds?: string[];
  onSubmit: (body: Record<string, unknown>) => void;
  submitting?: boolean;
  error?: string | null;
  idempotencyKey?: string;
}) {
  const [showBody, setShowBody] = useState(false);
  const issues = useMemo(() => validateCommand(draft, targetIds), [draft, targetIds]);
  const body = useMemo(() => buildCommand(draft), [draft]);
  const setLimits = (patch: Partial<CommandLimits>) => {
    if (draft.op === "crawl") {
      onChange({ ...draft, crawl: { ...draft.crawl, limits: { ...draft.crawl.limits, ...patch } } });
    } else if (draft.op === "export") {
      onChange({ ...draft, export: { ...draft.export, limits: { ...draft.export.limits, ...patch } } });
    } else {
      onChange({ ...draft, ai: { ...draft.ai, limits: { ...draft.ai.limits, ...patch } } });
    }
  };
  const limits =
    draft.op === "crawl"
      ? draft.crawl.limits
      : draft.op === "export"
        ? draft.export.limits
        : draft.ai.limits;

  return (
    <Card title="Compose a command">
      <div style={{ display: "grid", gap: 12 }}>
        <Field label="Operation" help="crawl reads the network, export renders the run output, ai asks for assistance">
          <Segmented
            ariaLabel="Operation"
            value={draft.op}
            onChange={(v) => onChange(emptyDraft(v as ControlOp, targetIds))}
            options={CONTROL_OPS.map((o) => ({ value: o, label: o }))}
          />
        </Field>

        {draft.op === "crawl" && (
          <Field
            label="Targets"
            help={
              targetIds.length === 0
                ? "no target list available (GET /config did not answer)"
                : `declared in the attached config: ${targetIds.join(", ")}`
            }
          >
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
              {(targetIds.length > 0 ? targetIds : ["(none)"]).map((t) => {
                const on = draft.crawl.targets.includes(t);
                return (
                  <button
                    key={t}
                    type="button"
                    aria-pressed={on}
                    className={on ? "chip" : "chip"}
                    disabled={targetIds.length === 0}
                    onClick={() =>
                      onChange({
                        ...draft,
                        crawl: {
                          ...draft.crawl,
                          targets: on
                            ? draft.crawl.targets.filter((x) => x !== t)
                            : [...draft.crawl.targets, t],
                        },
                      })
                    }
                  >
                    {t}
                  </button>
                );
              })}
            </div>
          </Field>
        )}

        {draft.op === "export" && (
          <>
            <Field label="Formats" help="each one is rendered by the engine's exporter, not by the console">
              <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                {EXPORT_FORMATS.map((f) => {
                  const on = draft.export.formats.includes(f);
                  return (
                    <button
                      key={f}
                      type="button"
                      aria-pressed={on}
                      className="chip"
                      onClick={() =>
                        onChange({
                          ...draft,
                          export: {
                            ...draft.export,
                            formats: on
                              ? draft.export.formats.filter((x) => x !== f)
                              : [...draft.export.formats, f],
                          },
                        })
                      }
                    >
                      {f}
                    </button>
                  );
                })}
              </div>
            </Field>
            <Field label="Dataset name" help="optional: prefixes the delivered files">
              <TextInput
                aria-label="Dataset name"
                value={draft.export.dataset}
                placeholder="(default: records)"
                onChange={(e) => onChange({ ...draft, export: { ...draft.export, dataset: e.target.value } })}
              />
            </Field>
            <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
              <Switch
                label="Bundle as zip"
                checked={draft.export.bundle}
                onChange={(v) => onChange({ ...draft, export: { ...draft.export, bundle: v } })}
              />
              <Switch
                label="Verify the delivery"
                checked={draft.export.verify}
                onChange={(v) => onChange({ ...draft, export: { ...draft.export, verify: v } })}
              />
            </div>
            <Field label="Record cap" help="0 = no cap beyond the limit below">
              <NumberInput
                aria-label="Record cap"
                value={draft.export.max_records}
                min={0}
                max={CEILINGS.maxRecords}
                onValue={(v) => onChange({ ...draft, export: { ...draft.export, max_records: v ?? 0 } })}
              />
            </Field>
          </>
        )}

        {draft.op === "ai" && (
          <>
            <div style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
              <Field label="Task">
                <SelectInput
                  aria-label="Task"
                  value={draft.ai.task}
                  onChange={(e) => onChange({ ...draft, ai: { ...draft.ai, task: e.target.value } })}
                >
                  {AI_TASKS.map((t) => (
                    <option key={t} value={t}>{t}</option>
                  ))}
                </SelectInput>
              </Field>
              <Field label="Provider" help="only providers this build registers can be named">
                <SelectInput
                  aria-label="Provider"
                  value={draft.ai.provider}
                  onChange={(e) => onChange({ ...draft, ai: { ...draft.ai, provider: e.target.value } })}
                >
                  {AI_PROVIDERS.map((p) => (
                    <option key={p} value={p}>{p}</option>
                  ))}
                </SelectInput>
              </Field>
              <Field label="Confidence" help="suggestions below this are reported, never applied">
                <NumberInput
                  aria-label="Confidence"
                  value={draft.ai.confidence}
                  min={0}
                  max={1}
                  step={0.05}
                  onValue={(v) => onChange({ ...draft, ai: { ...draft.ai, confidence: v ?? 0.9 } })}
                />
              </Field>
            </div>
            <div style={{ display: "flex", gap: 16, flexWrap: "wrap" }}>
              <Switch
                label="Consent to an external provider"
                checked={draft.ai.external_allowed}
                onChange={(v) => onChange({ ...draft, ai: { ...draft.ai, external_allowed: v } })}
              />
              <Field label="Source">
                <Segmented
                  ariaLabel="AI source"
                  value={draft.ai.source_kind}
                  onChange={(v) =>
                    onChange({ ...draft, ai: { ...draft.ai, source_kind: v as "run_output" | "inline" } })
                  }
                  options={[
                    { value: "run_output", label: "run output" },
                    { value: "inline", label: "inline fields" },
                  ]}
                />
              </Field>
            </div>
            {draft.ai.external_allowed && (
              <Note message="Consent is recorded in the audit log. It cannot summon a provider this build does not register — the only provider here is local." />
            )}
          </>
        )}

        <FormSection id="limits" title="Limits" count={6} defaultOpen={false}>
          <div style={{ display: "grid", gap: 10, gridTemplateColumns: "repeat(auto-fit,minmax(180px,1fr))" }}>
            <Field label="max_pages" help={`ceiling ${CEILINGS.maxPages.toLocaleString()}`}>
              <NumberInput
                aria-label="max_pages"
                value={limits.max_pages}
                min={1}
                max={CEILINGS.maxPages}
                onValue={(v) => setLimits({ max_pages: v ?? 0 })}
              />
            </Field>
            <Field label="max_depth" help={`ceiling ${CEILINGS.maxDepth}`}>
              <NumberInput
                aria-label="max_depth"
                value={limits.max_depth}
                min={0}
                max={CEILINGS.maxDepth}
                onValue={(v) => setLimits({ max_depth: v ?? 0 })}
              />
            </Field>
            <Field label="max_records" help={`ceiling ${CEILINGS.maxRecords.toLocaleString()}`}>
              <NumberInput
                aria-label="max_records"
                value={limits.max_records}
                min={1}
                max={CEILINGS.maxRecords}
                onValue={(v) => setLimits({ max_records: v ?? 0 })}
              />
            </Field>
            <Field label="max_export_bytes" help={`ceiling ${CEILINGS.maxExportBytes.toLocaleString()}`}>
              <NumberInput
                aria-label="max_export_bytes"
                value={limits.max_export_bytes}
                min={1}
                max={CEILINGS.maxExportBytes}
                onValue={(v) => setLimits({ max_export_bytes: v ?? 0 })}
              />
            </Field>
            <Field label="max_ai_input_bytes" help={`ceiling ${CEILINGS.maxAiInputBytes.toLocaleString()}`}>
              <NumberInput
                aria-label="max_ai_input_bytes"
                value={limits.max_ai_input_bytes}
                min={1}
                max={CEILINGS.maxAiInputBytes}
                onValue={(v) => setLimits({ max_ai_input_bytes: v ?? 0 })}
              />
            </Field>
            <Field label="timeout_secs" help={`ceiling ${CEILINGS.timeoutSecs}`}>
              <NumberInput
                aria-label="timeout_secs"
                value={limits.timeout_secs}
                min={1}
                max={CEILINGS.timeoutSecs}
                onValue={(v) => setLimits({ timeout_secs: v ?? 0 })}
              />
            </Field>
          </div>
        </FormSection>

        {issues.length > 0 && (
          <div role="alert" style={{ display: "grid", gap: 4 }}>
            {issues.map((i) => (
              <Note key={`${i.field}:${i.message}`} message={`${i.field}: ${i.message}`} />
            ))}
          </div>
        )}
        {error && <Note message={error} />}

        <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
          <Button
            variant="primary"
            disabled={submitting || issues.length > 0}
            onClick={() => onSubmit(body)}
          >
            {submitting ? "Submitting…" : `Submit ${draft.op} command`}
          </Button>
          <Button variant="ghost" onClick={() => setShowBody((v) => !v)}>
            {showBody ? "Hide" : "Show"} request body
          </Button>
          {idempotencyKey && (
            <span className="t-caption">
              Idempotency-Key{" "}
              <code className="t-code">{idempotencyKey}</code>{" "}
              <CopyButton text={idempotencyKey} label="Copy key" />
            </span>
          )}
        </div>
        {showBody && (
          <pre className="t-code" style={{ margin: 0, overflowX: "auto" }}>
            {JSON.stringify(body, null, 2)}
          </pre>
        )}
      </div>
    </Card>
  );
}

export default CommandComposer;
