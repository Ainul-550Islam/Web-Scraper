// keys: barrel — one file per unit; every prior import path & name still resolves

/**
 * The managed API's key surface.
 *
 * `ApiKeys` is the page; `KeysClient` is the typed operator client for
 * `/v1/admin/*`; `types` holds the wire shapes. The plaintext of a freshly
 * issued key lives in `KeyReveal`'s state for as long as that dialog is open
 * and nowhere else.
 */
export * from "./types";
export * from "./KeysClient";
export * from "./CreateKeyForm";
export * from "./KeyReveal";
export { ApiKeys } from "./ApiKeys";
