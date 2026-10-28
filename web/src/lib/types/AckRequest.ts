// AckRequest — split from types.ts; barrel re-exports keep every public path & name



/* Alert-ack POST body; bounded control-plane command shapes are modeled separately. */
export interface AckRequest {
  id: string; // correlation id or "*" (bulk)
}
