// Alerts: barrel — one file per unit; every prior import path & name still resolves

/**
 * `GET /alerts` + `POST /alerts/ack?event=<name>` (or {"event"} body — the
 * server accepts both; we use the query form, validation is correlation-id
 * charset + a literal "*"). Bulk ack is confirmed twice over: modal + the
 * server answers with the number of entries actually acknowledged.
 */
export * from "./Alerts";
export { default } from "./Alerts";
