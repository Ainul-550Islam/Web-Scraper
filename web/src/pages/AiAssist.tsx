/**
 * AI Assist — review surface for the optional AI stage.
 *
 * Three facts drive the whole page, and it states all three:
 *
 *  1. **AI is optional and off by default.** With no provider document loaded
 *     the page says "disabled" and shows no suggestions. It never invents one.
 *  2. **External providers need explicit consent.** `external_allowed` is
 *     false unless the operator opts in; a provider marked external without
 *     consent is shown as blocked, not as working.
 *  3. **Assisted is not validated.** Every card carries an `AI SUGGESTION`
 *     badge until a human confirms it; deterministic values say `VALIDATED`.
 *
 * Provider status and suggestions come from documents you load (the operator
 * configuration and the stage's output) — the console has no AI endpoint to
 * call, and it does not pretend to have one.
 */
import { useMemo, useState } from "react";
import { Card, CopyButton, EmptyState, MetricCard, PageHeader, StatusChip, Toggle } from "../components/ui";
import { Page } from "../components/AppShell";
import AiSuggestionCard, { bandOf, provenanceLabel, type AiSuggestion, type ReviewState } from "../components/AiSuggestionCard";
import { JsonInspector } from "../components/patterns/JsonInspector";
import { readText } from "../features/delivery/Delivery";
import { fmtNum } from "../lib/format";
import { REDACTION_MARK } from "../lib/redaction";

/** Mirrors `harvest::ai::AiCfg` (src/ai.rs). */
export interface AiProviderDoc {
  enabled: boolean;
  external_allowed?: boolean;
  provider?: string;
  model?: string | null;
  timeout_ms?: number;
  max_retries?: number;
  request_budget?: number;
  input_budget_bytes?: number;
  thresholds?: { accept?: number; review?: number };
  overwrite_validated?: boolean;
  redact_fields?: string[];
}

export interface AiOutcomeDoc {
  state: string;
  suggestions?: AiSuggestion[];
  candidates?: AiSuggestion[];
  provider?: { name?: string; model?: string | null; latency_ms?: number | null };
  requests_used?: number;
}

export interface ProviderStatus {
  enabled: boolean;
  configured: boolean;
  external: boolean;
  /** true when an external provider is allowed to be called */
  consentOk: boolean;
  name: string;
  model: string | null;
  thresholds: { accept: number; review: number };
  budget: { used: number; max: number };
  overwriteValidated: boolean;
}

/** External providers carry data out of the process — that needs saying. */
const KNOWN_EXTERNAL = /^(openai|anthropic|azure|vertex|bedrock|ollama-remote|http)/i;

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function providerStatus(doc: AiProviderDoc | null, used = 0): ProviderStatus {
  if (!doc) {
    return {
      enabled: false,
      configured: false,
      external: false,
      consentOk: false,
      name: "disabled",
      model: null,
      thresholds: { accept: 0.9, review: 0.6 },
      budget: { used: 0, max: 0 },
      overwriteValidated: false,
    };
  }
  const provider = doc.provider ?? "";
  const external = KNOWN_EXTERNAL.test(provider) || provider.includes("://");
  return {
    enabled: doc.enabled,
    configured: doc.enabled && provider.length > 0 && provider !== "disabled",
    external,
    consentOk: !external || !!doc.external_allowed,
    name: provider || "unset",
    model: doc.model ?? null,
    thresholds: { accept: doc.thresholds?.accept ?? 0.9, review: doc.thresholds?.review ?? 0.6 },
    budget: { used, max: doc.request_budget ?? 0 },
    overwriteValidated: !!doc.overwrite_validated,
  };
}

export default function AiAssist() {
  const [doc, setDoc] = useState<AiProviderDoc | null>(null);
  const [docError, setDocError] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<AiOutcomeDoc | null>(null);
  const [outcomeError, setOutcomeError] = useState<string | null>(null);
  const [review, setReview] = useState<Record<string, ReviewState>>({});
  const [showRaw, setShowRaw] = useState(false);

  const status = useMemo(() => providerStatus(doc, outcome?.requests_used ?? 0), [doc, outcome]);
  const suggestions = useMemo(() => {
    const all = [...(outcome?.suggestions ?? []), ...(outcome?.candidates ?? [])];
    return all.filter((s) => s && typeof s.field === "string");
  }, [outcome]);

  const counts = useMemo(() => {
    const c = { high: 0, medium: 0, low: 0 };
    for (const s of suggestions) c[bandOf(s.confidence)] += 1;
    return c;
  }, [suggestions]);

  async function loadDoc(file: File) {
    setDocError(null);
    try {
      const parsed = JSON.parse(await readText(file)) as AiProviderDoc;
      if (typeof parsed !== "object" || parsed === null || typeof parsed.enabled !== "boolean") {
        throw new Error("expected { enabled: boolean, provider: string, … }");
      }
      setDoc(parsed);
    } catch (e) {
      setDoc(null);
      setDocError(e instanceof Error ? e.message : String(e));
    }
  }

  async function loadOutcome(file: File) {
    setOutcomeError(null);
    setReview({});
    try {
      const parsed = JSON.parse(await readText(file)) as AiOutcomeDoc;
      if (typeof parsed !== "object" || parsed === null || typeof parsed.state !== "string") {
        throw new Error("expected { state: string, suggestions?: […] }");
      }
      setOutcome(parsed);
    } catch (e) {
      setOutcome(null);
      setOutcomeError(e instanceof Error ? e.message : String(e));
    }
  }

  const setReviewFor = (field: string, next: ReviewState) => setReview((prev) => ({ ...prev, [field]: next }));

  const blocked = status.enabled && status.configured && !status.consentOk;

  return (
    <Page>
      <PageHeader
        title="AI Assist"
        sub="Optional, explicit, budgeted assistance for extraction and normalization — off unless a provider is configured, and never authoritative."
      />

      <div className="grid grid-metrics">
        <MetricCard
          label="Provider"
          value={<StatusChip status={status.configured ? "ok" : "disabled"} dot label={status.configured ? "configured" : "disabled"} />}
          meta={status.name}
        />
        <MetricCard
          label="External calls"
          value={<StatusChip status={blocked ? "error" : status.external ? (status.consentOk ? "ok" : "disabled") : "idle"} dot label={blocked ? "blocked — no consent" : status.external ? (status.consentOk ? "consented" : "not consented") : "none (local only)"} />}
          meta={`external_allowed: ${status.consentOk}`}
        />
        <MetricCard
          label="Requests"
          value={fmtNum(status.budget.used)}
          unit={status.budget.max > 0 ? `/ ${fmtNum(status.budget.max)}` : undefined}
          meta={status.budget.max > 0 ? "budget for this run" : "no budget configured"}
        />
        <MetricCard
          label="Thresholds"
          value={`≥ ${status.thresholds.accept.toFixed(2)} accept`}
          meta={`≥ ${status.thresholds.review.toFixed(2)} review · below → reject`}
        />
      </div>

      <Card title="Provider configuration">
        <div className="ai-pick">
          <label className="ai-label">
            <span>provider document (ai config JSON)</span>
            <input type="file" accept=".json,application/json" aria-label="provider document" onChange={(e) => { const f = e.target.files?.[0]; if (f) void loadDoc(f); }} />
          </label>
          <label className="ai-label">
            <span>suggestion output (ai stage JSON)</span>
            <input type="file" accept=".json,application/json" aria-label="suggestion document" onChange={(e) => { const f = e.target.files?.[0]; if (f) void loadOutcome(f); }} />
          </label>
        </div>
        {docError ? <p className="t-caption t-err">provider document rejected: {docError}</p> : null}
        {outcomeError ? <p className="t-caption t-err">suggestion document rejected: {outcomeError}</p> : null}

        <dl className="ai-facts">
          <div>
            <dt>state</dt>
            <dd>{status.configured ? (blocked ? "blocked (external consent missing)" : "configured") : "disabled by default"}</dd>
          </div>
          <div>
            <dt>provider / model</dt>
            <dd>
              {status.name}
              {status.model ? ` · ${status.model}` : ""}
            </dd>
          </div>
          <div>
            <dt>overwrite validated values</dt>
            <dd>{status.overwriteValidated ? "allowed by configuration" : "refused (default)"}</dd>
          </div>
          <div>
            <dt>redaction</dt>
            <dd>credential-shaped fields are replaced with {REDACTION_MARK} before any provider request is built</dd>
          </div>
        </dl>

        {blocked ? (
          <p className="t-caption t-err">
            This provider is external and <code>external_allowed</code> is false. The engine refuses to send data out of
            the process until that is explicitly enabled — the console cannot override it.
          </p>
        ) : null}
        {doc ? (
          <div className="ai-raw">
            <Toggle checked={showRaw} onChange={setShowRaw} label="show raw provider document" />
            {showRaw ? <JsonInspector value={doc} maxNodes={200} openDepth={1} redact /> : null}
          </div>
        ) : (
          <EmptyState
            title="No provider configured"
            body="Without a provider document the AI stage is disabled — that is the default and the safe state. Load one to review what it proposes."
          />
        )}
      </Card>

      <Card
        title="Suggestions"
        actions={
          <span className="t-caption t-muted">
            {fmtNum(counts.high)} high · {fmtNum(counts.medium)} medium · {fmtNum(counts.low)} low
          </span>
        }
      >
        {!status.configured ? (
          <EmptyState
            title="AI disabled — no suggestions"
            body="Deterministic extraction is unaffected. Nothing on this page will invent an AI result while the stage is off."
          />
        ) : suggestions.length === 0 ? (
          <EmptyState
            title="No suggestions in this document"
            body={`Stage state: ${outcome?.state ?? "unknown"}. An empty or failed stage is reported as empty — never backfilled.`}
          />
        ) : (
          <div className="ai-grid">
            {suggestions.map((s, i) => (
              <AiSuggestionCard
                key={`${s.field}-${i}`}
                suggestion={s}
                review={review[s.field] ?? "pending"}
                onAccept={(x) => setReviewFor(x.field, "accepted")}
                onReject={(x) => setReviewFor(x.field, "rejected")}
                onReview={(x) => setReviewFor(x.field, "review")}
              />
            ))}
          </div>
        )}

        {suggestions.length > 0 ? (
          <div className="ai-footer">
            <p className="t-caption t-muted">
              Every card above is labelled {provenanceLabel("ai-suggestion")} until a human accepts it; accepted cards
              become {provenanceLabel("ai-confirmed")}. Deterministic extraction values read {provenanceLabel("deterministic")}
              {" "}and are never overwritten by a suggestion unless the configuration allows it.
            </p>
            <CopyButton
              text={JSON.stringify(suggestions.map((s) => ({ field: s.field, review: review[s.field] ?? "pending" })), null, 2)}
              label="Copy review decisions"
            />
          </div>
        ) : null}
      </Card>

      <style>{`
        .ai-pick { align-items: flex-end; display: flex; flex-wrap: wrap; gap: 18px; }
        .ai-label { display: flex; flex-direction: column; font-size: 12px; gap: 6px; }
        .ai-facts { display: grid; gap: 10px; grid-template-columns: repeat(auto-fit, minmax(230px, 1fr)); margin: 14px 0 0; }
        .ai-facts dt { color: var(--ink-faint); font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; }
        .ai-facts dd { font-size: 12px; margin: 2px 0 0; }
        .ai-raw { align-items: center; display: flex; flex-direction: column; gap: 8px; margin-top: 12px; align-items: flex-start; }
        .ai-grid { display: grid; gap: 12px; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); }
        .ai-footer { align-items: center; display: flex; flex-wrap: wrap; gap: 12px; justify-content: space-between; margin-top: 12px; }
      `}</style>
    </Page>
  );
}
