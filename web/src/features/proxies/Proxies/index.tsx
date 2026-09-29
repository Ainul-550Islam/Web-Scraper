// Proxies: barrel — one file per unit; every prior import path & name still resolves

/** GET /proxies snapshot shape (superset of the /stats proxy rows). */
export * from "./ProxyPoolEntry";
export * from "./ProxyPoolResponse";
export * from "./Proxies";
export { default } from "./Proxies";
