// connection: barrel — one file per unit; every prior import path & name still resolves

/**
 * Shell-level connection truth: health + config + alert count, one shared
 * poller so every page reads the same snapshot (no per-component storms).
 * Everything a page needs that is NOT here comes from its own endpoint —
 * the console never invents a field the server does not send.
 */
export * from "./ConnectionState";
export * from "./Ctx";
export * from "./Bundle";
export * from "./ConnectionProvider";
export * from "./useConnection";
