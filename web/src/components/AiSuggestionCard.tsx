/**
 * AiSuggestionCard — one AI proposal, labelled as one.
 *
 * The card exists to make the provenance of a value impossible to misread:
 * every suggestion renders a `AI SUGGESTION` badge until a human (or an
 * explicit policy) confirms it, and only then does it become `AI CONFIRMED`.
 * Deterministic values render `VALIDATED`. There is no visual state in which
 * an unconfirmed suggestion looks like verified data.
 *
 * Nothing here can display a credential: values are redacted by the caller
 * (`deepMask`) and the card refuses to render raw text for fields whose name
 * looks sensitive.
 */
import { REDACTION_MARK } from "../lib/redaction";
import { StatusChip, Button } from "./ui";

/** Shape produced by `harvest::ai::schema::FieldSuggestion::to_json`. */
export interface AiSuggestion {
  field: string;
  value: unknown;
  suggested: string;
  /** 0..1 */
  confidence: number;
  rationale: string;
  /** "deterministic" | "ai-suggestion" | "ai-confirmed" */
  provenance: string;
  replaces_validated?: boolean;
}

export interface Props {
  suggestion: AiSuggestion;
  /** "pending" | "accepted" | "rejected" | "review" */
  review?: ReviewState;
  onAccept?: (s: AiSuggestion) => void;
  onReject?: (s: AiSuggestion) => void;
  onReview?: (s: AiSuggestion) => void;
}

export type ReviewState = "pending" | "accepted" | "rejected" | "review";

/**
 * Confidence bands, matching `harvest::ai::schema` exactly: >= 0.90 high,
 * >= 0.60 medium, below that low. Keeping the numbers here (rather than
 * deriving them) means the console's thresholds cannot drift from the Rust
 * layer's without a test failing.
 */
export type ConfidenceBand = "high" | "medium" | "low";
export const CONFIDENCE_HIGH = 0.9;
export const CONFIDENCE_MEDIUM = 0.6;

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function bandOf(confidence: number): ConfidenceBand {
  if (!Number.isFinite(confidence)) return "low";
  if (confidence >= CONFIDENCE_HIGH) return "high";
  if (confidence >= CONFIDENCE_MEDIUM) return "medium";
  return "low";
}

const SENSITIVE = /(token|authoriz|password|passwd|secret|api[_-]?key|credential|cookie|session)/i;

/** The label a reader must see: assisted is never presented as verified. */
// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function provenanceLabel(provenance: string): string {
  switch (provenance) {
    case "deterministic":
      return "VALIDATED";
    case "ai-confirmed":
      return "AI CONFIRMED";
    case "ai-suggestion":
      return "AI SUGGESTION";
    default:
      return "UNKNOWN PROVENANCE";
  }
}

// shared with the page and the tests — this module owns the value.
// eslint-disable-next-line react-refresh/only-export-components
export function displayValue(field: string, value: unknown): string {
  if (SENSITIVE.test(field)) return REDACTION_MARK;
  if (value === null || value === undefined) return "—";
  if (typeof value === "string") return value.length > 120 ? `${value.slice(0, 120)}…` : value;
  return JSON.stringify(value) ?? "—";
}

export default function AiSuggestionCard({ suggestion, review = "pending", onAccept, onReject, onReview }: Props) {
  const band: ConfidenceBand = bandOf(suggestion.confidence);
  const label = provenanceLabel(suggestion.provenance);
  const isAi = suggestion.provenance !== "deterministic";
  const tone = band === "high" ? "ok" : band === "medium" ? "warn" : "error";

  return (
    <article className={`ai-card${isAi ? " ai-assisted" : ""}`} data-field={suggestion.field} data-band={band}>
      <header className="ai-head">
        <code className="ai-field">{suggestion.field}</code>
        <span className={`ai-badge ${isAi ? "assisted" : "validated"}`} data-testid="provenance-label">
          {label}
        </span>
      </header>

      <div className="ai-body">
        <div className="ai-value" title={String(displayValue(suggestion.field, suggestion.value))}>
          {displayValue(suggestion.field, suggestion.value)}
        </div>
        <div className="ai-meta">
          <span className="ai-type">{suggestion.suggested}</span>
          <span className="ai-conf">
            <span className={`ai-bar ${band}`} aria-hidden="true">
              <span style={{ width: `${Math.round(suggestion.confidence * 100)}%` }} />
            </span>
            <span className="ai-conf-num">{suggestion.confidence.toFixed(2)}</span>
            <StatusChip status={tone} label={band} />
          </span>
        </div>
      </div>

      <p className="ai-why">{suggestion.rationale}</p>
      {suggestion.replaces_validated ? (
        <p className="t-caption t-warn">
          Would replace an existing deterministic value — accepting needs an explicit overwrite policy.
        </p>
      ) : null}

      <footer className="ai-actions">
        <StatusChip status={review === "accepted" ? "ok" : review === "rejected" ? "error" : "queued"} dot label={`review: ${review}`} />
        <span className="ai-btns">
          {onAccept ? (
            <Button variant="ghost" className="sm" onClick={() => onAccept(suggestion)}>
              Accept
            </Button>
          ) : null}
          {onReview ? (
            <Button variant="ghost" className="sm" onClick={() => onReview(suggestion)}>
              Mark for review
            </Button>
          ) : null}
          {onReject ? (
            <Button variant="ghost" className="sm" onClick={() => onReject(suggestion)}>
              Reject
            </Button>
          ) : null}
        </span>
      </footer>

      <style>{`
        .ai-card { background: var(--surface-2); border: 1px solid var(--line); border-radius: var(--r-md);
          display: flex; flex-direction: column; gap: 10px; padding: 14px; }
        .ai-card.ai-assisted { border-left: 3px solid var(--warn); }
        .ai-head { align-items: center; display: flex; gap: 10px; justify-content: space-between; }
        .ai-field { font-size: 12px; }
        .ai-badge { border-radius: 999px; font-size: 10px; font-weight: 700; letter-spacing: 0.08em; padding: 3px 8px; }
        .ai-badge.assisted { background: var(--warn-soft); color: var(--warn-ink); }
        .ai-badge.validated { background: var(--ok-soft); color: var(--ok-ink); }
        .ai-body { display: flex; flex-direction: column; gap: 8px; }
        .ai-value { font-size: 13px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
        .ai-meta { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; justify-content: space-between; }
        .ai-type { color: var(--ink-dim); font-size: 11px; }
        .ai-conf { align-items: center; display: flex; gap: 8px; }
        .ai-bar { background: var(--line); border-radius: 999px; height: 4px; overflow: hidden; width: 84px; }
        .ai-bar > span { display: block; height: 100%; }
        .ai-bar.high > span { background: var(--ok); }
        .ai-bar.medium > span { background: var(--warn); }
        .ai-bar.low > span { background: var(--err); }
        .ai-conf-num { font-size: 11px; font-variant-numeric: tabular-nums; }
        .ai-why { color: var(--ink-mute); font-size: 11px; margin: 0; }
        .ai-actions { align-items: center; display: flex; flex-wrap: wrap; gap: 10px; justify-content: space-between; }
        .ai-btns { display: flex; gap: 6px; }
      `}</style>
    </article>
  );
}
