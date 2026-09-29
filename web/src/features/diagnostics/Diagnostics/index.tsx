// Diagnostics: barrel — one file per unit; every prior import path & name still resolves

/**
 * GET /diagnostics expanded card-by-card, plus the shell's own live facts
 * (rtt, cadence). "Copy diagnostic report" serializes ONLY real, already
 * redacted data — with a defensive `maskSecrets` pass layered on top so a
 * future server field can't leak through a copy/paste.
 */
export * from "./Diagnostics";
export * from "./formatVal";
export { default } from "./Diagnostics";
