// ApiRoute — the one row shape the API Explorer and the contract tests share.

export interface ApiRoute {
  method: "GET" | "POST";
  path: string;
  /** "always-open" routes answer without credentials; everything else is
   * bearer-gated once `admin_token` is configured (loopback may stay open). */
  auth: "always-open" | "bearer-if-configured";
  summary: string;
  /** query params the engine honors (from src/admin.rs parsing), if any */
  params?: string;
  /** doc file in the repo that owns this surface (docs/api/NN-*.md) */
  doc?: string;
}
