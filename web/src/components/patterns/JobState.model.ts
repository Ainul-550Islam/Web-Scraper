// Pure control-job state vocabulary shared by pages and presentation components.

export const JOB_STATES = ["queued", "running", "completed", "failed", "cancelled"] as const;
export type JobState = (typeof JOB_STATES)[number];

export function isJobState(s: string): s is JobState {
  return (JOB_STATES as readonly string[]).includes(s);
}

/** Terminal states are the ones the server will not move a job out of. */
export function isTerminal(state: string): boolean {
  return state === "completed" || state === "failed" || state === "cancelled";
}

/** Whether a cancel request can still do anything. */
export function canCancel(state: string): boolean {
  return state === "queued" || state === "running";
}

export const BLURB: Record<JobState, string> = {
  queued: "accepted, waiting for a slot",
  running: "executing under its deadline",
  completed: "finished; result retained",
  failed: "finished with a reason",
  cancelled: "stopped at the operator's request",
};

export function jobBlurb(state: string): string {
  return isJobState(state) ? BLURB[state] : "unknown state";
}

