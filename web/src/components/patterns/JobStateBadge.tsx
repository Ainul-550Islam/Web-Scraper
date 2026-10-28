// JobStateBadge — render the control plane's authoritative job state vocabulary.

import { StatusChip } from "../ui";
import { BLURB, JOB_STATES, isTerminal } from "./JobState.model";

export function JobStateBadge({
  state,
  cancelRequested = false,
  title,
}: {
  state: string;
  cancelRequested?: boolean;
  title?: string;
}) {
  const cancelling = cancelRequested && !isTerminal(state);
  return (
    <span style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
      <StatusChip status={state} />
      {cancelling && (
        <span className="t-caption" title="the cancel request was accepted; the job stops at its next checkpoint">
          cancelling…
        </span>
      )}
      {title && <span className="t-caption">{title}</span>}
    </span>
  );
}

/** One row of the state legend, so every page explains the same vocabulary. */
export function JobStateLegend() {
  return (
    <div style={{ display: "flex", gap: 10, flexWrap: "wrap", alignItems: "center" }}>
      {JOB_STATES.map((s) => (
        <span key={s} style={{ display: "inline-flex", gap: 5, alignItems: "center" }}>
          <StatusChip status={s} />
          <span className="t-caption">{BLURB[s]}</span>
        </span>
      ))}
    </div>
  );
}

export default JobStateBadge;
