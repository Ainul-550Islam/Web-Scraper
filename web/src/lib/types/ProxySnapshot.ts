// ProxySnapshot — split from types.ts; barrel re-exports keep every public path & name



export interface ProxySnapshot {
  id: string;
  requests: number;
  failures: number;
  inflight: number;
  latency_ewma_ms: number;
  success_ewma: number;
  region: string | null;
}
