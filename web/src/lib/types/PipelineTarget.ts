// PipelineTarget — split from types.ts; barrel re-exports keep every public path & name



/* ── pipeline / storage / diagnostics ─────────────────────────── */
export interface PipelineTarget {
  target: string;
  enforce: string;
  schema_fields: number;
  transform_ops: number;
}
