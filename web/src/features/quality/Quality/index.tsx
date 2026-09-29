// Quality: barrel — one file per unit; every prior import path & name still resolves

/**
 * Data Quality from REAL cumulative counters on GET /stats + the per-run
 * contract tally on GET /pipeline. The funnel below is sequential because
 * the engine's stages ARE sequential (fetch → extract → dedupe → write);
 * each "loss" row shows its counter, never a delta estimate.
 */
export * from "./Quality";
export { default } from "./Quality";
