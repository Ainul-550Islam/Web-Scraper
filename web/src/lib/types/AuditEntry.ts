// AuditEntry — split from types.ts; barrel re-exports keep every public path & name


export interface AuditEntry {
  ts?: number;
  action?: string;
  result?: string;
  corr?: string;
  [k: string]: unknown;
}
