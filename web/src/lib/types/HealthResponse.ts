// HealthResponse — split from types.ts; barrel re-exports keep every public path & name


export interface HealthResponse {
  status: "ok";
  running: boolean;
}
