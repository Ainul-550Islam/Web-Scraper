// TransformArg — split from configSchema.ts; barrel re-exports keep every public path & name



/**
 * Exact argument surface of every transform, mirroring the Rust enum
 * variants (field names = serde snake_case of the struct members).
 */
export interface TransformArg {
  name: string;
  kind: "string" | "number" | "bool" | "list";
  required: boolean;
  default?: unknown;
  options?: string[];
}
