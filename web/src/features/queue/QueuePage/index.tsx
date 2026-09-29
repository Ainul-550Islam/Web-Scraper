// QueuePage: barrel — one file per unit; every prior import path & name still resolves

/**
 * `GET /queue` — real component health + live queue stats. Charts only use
 * actual per-target counts the API returns (`per_target_depths/active`).
 */
export * from "./QueuePage";
export { default } from "./QueuePage";
