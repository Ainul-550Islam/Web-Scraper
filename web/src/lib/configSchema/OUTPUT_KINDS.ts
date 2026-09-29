// OUTPUT_KINDS — split from configSchema.ts; barrel re-exports keep every public path & name



export const OUTPUT_KINDS = [
  "jsonl",
  "csv",
  "sqlite",
  "postgres",
  "http",
  "dataset",
  "null",
] as const;
