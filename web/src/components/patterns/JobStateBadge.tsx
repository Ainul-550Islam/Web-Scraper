// JobStateBadge — the control plane's five job states, in one place.
//
// The states are the backend's (`harvest::execution::ExecState`) and the
// transitions are the backend's too (`ExecState::can_transition`): the console
// never invents a state, and it never shows one that the server did not send.
// `cancelling` is the one client-side nuance — it marks the window between an
// accepted cancel request and the server reporting the job terminal, and it is
// always shown *next to* the server's state, never instead of it.

import { StatusChip } from "../ui";

export const JOB_STATES = ["queued", "running", "completed", "failed", "cancelled"] as const;
export type JobState = (typeof JOB_STATES)[number];

// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function isJobState(s: string): s is JobState {
  return (JOB_STATES as readonly string[]).includes(s);
}

/** Terminal states are the ones the server will not move a job out of. */
// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function isTerminal(state: string): boolean {
  return state === "completed" || state === "failed" || state === "cancelled";
}

/** Whether a cancel request can still do anything. */
// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function canCancel(state: string): boolean {
  return state === "queued" || state === "running";
}

const BLURB: Record<JobState, string> = {
  queued: "accepted, waiting for a slot",
  running: "executing under its deadline",
  completed: "finished; result retained",
  failed: "finished with a reason",
  cancelled: "stopped at the operator's request",
};

// eslint-disable-next-line react-refresh/only-export-components -- a shared helper lives with the component it serves
export function jobBlurb(state: string): string {
  return isJobState(state) ? BLURB[state] : "unknown state";
}

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
