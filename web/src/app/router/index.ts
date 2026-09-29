// router: barrel — one file per unit; every prior import path & name still resolves

/**
 * Hash router — deliberate: the console is frequently reached through an
 * embedded admin port where SPA history rewrites are unavailable. `#/route`
 * survives any static server, and links stay shareable/copyable.
 */
export * from "./RouteState";
export * from "./parseHash";
export * from "./navigate";
export * from "./useRoute";
