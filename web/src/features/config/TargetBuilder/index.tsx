// TargetBuilder: barrel — one file per unit; every prior import path & name still resolves

/**
 * Per-target editor over the real TargetCfg shape (GET /config → targets).
 * The stage strip at the top is the ACTUAL processing order (engine chain);
 * every control writes to the in-memory draft only.
 */
export * from "./TargetBuilder";
export * from "./TransformEntry";
export * from "./emptyArgs";
export * from "./argsReady";
export * from "./TransformList";
export * from "./TagInputLike";
