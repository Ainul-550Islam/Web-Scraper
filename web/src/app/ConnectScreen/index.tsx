// ConnectScreen: barrel — one file per unit; every prior import path & name still resolves

/**
 * First-run connection experience (§3). Shown only when the console has
 * never reached the API this session and the network itself is failing —
 * once any endpoint answers, the shell takes over and this never blocks
 * again.
 *
 * Honest by construction: it displays product identity and connection
 * facts, and it deliberately shows NO version/build banner, because the
 * admin API (`/health`, `/runtime`) exposes none — fabricating one would
 * be the first lie in the product.
 */
export * from "./CAPABILITIES";
export * from "./ConnectScreen";
