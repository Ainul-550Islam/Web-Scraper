// ApiRoute — the one row shape the API Explorer and the contract tests share.

export interface ApiRoute {
  method: "GET" | "POST";
  path: string;
  /** "always-open" routes answer without credentials; "open-contract" is the
   * published OpenAPI document, which a client has to be able to read before
   * it knows which keys to ask for. Everything else is bearer-gated once
   * `admin_token` is configured (loopback may stay open); "api-key" routes
   * take a tenant api key instead of the console bearer. */
  auth: "always-open" | "open-contract" | "bearer-if-configured" | "api-key";
  summary: string;
  /** query params the engine honors (from src/admin.rs parsing), if any */
  params?: string;
  /** doc file in the repo that owns this surface (docs/api/NN-*.md) */
  doc?: string;
}
