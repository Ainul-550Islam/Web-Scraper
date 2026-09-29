// Pipeline: barrel — one file per unit; every prior import path & name still resolves

/**
 * Read-only view of the ACTUAL execution chain (engine.rs), not an editor:
 *   Target → Policy → Fetcher → Proxy → Extraction → Transform → Schema
 *   → Dedupe → Sink
 * Node metadata and counts all come from GET /config (redacted) and
 * GET /pipeline; clicking a node opens a panel with the same real data,
 * scoped to that stage.
 */
export * from "./Stage";
export * from "./Ctx";
export * from "./STAGES";
export * from "./Pipeline";
export * from "./stageTone";
export * from "./renderDetail";
export { default } from "./Pipeline";
