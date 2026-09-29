// Jobs: barrel — one file per unit; every prior import path & name still resolves

/**
 * `GET /jobs` is the per-target tally of the live run, falling back to the
 * newest persisted run (the API says so in `note`) — the UI shows exactly
 * that provenance and nothing else. Lifecycle/retry/failure detail joins the
 * live event ring by target index where present.
 */
export * from "./Jobs";
export { default } from "./Jobs";
