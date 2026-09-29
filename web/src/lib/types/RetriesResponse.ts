// RetriesResponse — split from types.ts; barrel re-exports keep every public path & name


export interface RetriesResponse {
  state: "available" | "unavailable";
  accounting_retries?: number;
  requests_retry?: number;
  recent_delays_ms: number[];
}
