// OpsEntry — split from types.ts; barrel re-exports keep every public path & name


export interface OpsEntry {
  ts: number;
  kind: string;
  detail: Record<string, unknown>;
}
