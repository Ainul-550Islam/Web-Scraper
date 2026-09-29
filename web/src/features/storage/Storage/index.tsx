// Storage: barrel — one file per unit; every prior import path & name still resolves

/**
 * `GET /storage` — one row per configured lane (path pre-masked by the
 * server as `…/rel/path`). Existence + byte size are lstat facts, not
 * estimates. SQL/redis backend availability is compiled-in truth from
 * `GET /config._backends`.
 */
export * from "./Storage";
export { default } from "./Storage";
