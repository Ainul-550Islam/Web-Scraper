// usePolled: barrel — one file per unit; every prior import path & name still resolves

/**
 * Controlled polling with the four states the status bar shows:
 *   live        — data present, next refresh scheduled
 *   refreshing  — data present, request in flight
 *   paused      — tab hidden (visibilitychange) or caller-paused
 *   offline     — network failure; last data (if any) kept for context
 *
 * Guards: overlapping ticks are skipped (never concurrent), obsolete
 * requests are aborted when superseded, and unmount cleans up fully.
 */
export * from "./PollState";
export * from "./Polled";
export * from "./usePolled";
export * from "./documentHidden";
