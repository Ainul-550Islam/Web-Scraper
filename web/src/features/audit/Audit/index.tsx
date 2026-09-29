// Audit: barrel — one file per unit; every prior import path & name still resolves

/**
 * Two real feeds, one explorer:
 *  • Audit journal  — GET /audit (HMAC-sealed `audit.jsonl`, only exists when
 *    `audit_path` is configured; empty is shown as empty, honestly)
 *  • Ops feed       — GET /ops (run lifecycle + operator notes, newest first,
 *    capped window)
 * Detail drawer prints the raw JSON entry (server-redacted) — never re-keys
 * hidden secrets.
 */
export * from "./Audit";
export * from "./toneKind";
export * from "./summarize";
export { default } from "./Audit";
