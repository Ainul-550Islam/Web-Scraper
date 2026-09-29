// mock: barrel — one file per unit; every prior import path & name still resolves

/**
 * DEV-ONLY mock mode — deterministic fixtures captured verbatim from a live
 * `harvest serve` on the bundled demo origin (see docs/web-client.md).
 *
 * Hard guard: mocks are reachable ONLY when `import.meta.env.DEV` is true
 * AND VITE_MOCK==="1". A production build returns null from every entry
 * point, so mock data can never appear in a live console by accident.
 */
export * from "./MOCK_ENABLED";
export * from "./TS";
export * from "./mockResponse";
export * from "./STATS";
export * from "./RUNTIME";
export * from "./JOBS";
export * from "./RUNS";
export * from "./RECENT";
export * from "./EVENTS";
export * from "./WORKERS";
export * from "./QUEUE";
export * from "./PROXIES";
export * from "./PIPELINE";
export * from "./STORAGE";
export * from "./DIAGNOSTICS";
export * from "./ALERTS";
export * from "./OPS";
export * from "./AUDIT";
export * from "./CONFIG";
