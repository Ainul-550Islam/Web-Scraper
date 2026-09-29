// TRANSFORM_OPS — split from configSchema.ts; barrel re-exports keep every public path & name



/* ── transform catalog (src/config.rs TransformCfg) ───────────── */
export const TRANSFORM_OPS = [
  "trim",
  "lowercase",
  "uppercase",
  "collapse_whitespace",
  "regex_replace",
  "regex_extract",
  "strip_tags",
  "mask_pii",
  "parse_number",
  "round_to",
  "units",
  "parse_date",
  "to_bool",
  "split",
  "join",
  "unique",
  "sort",
  "take",
  "drop",
  "truncate",
  "replace_empty",
  "prefix",
  "suffix",
] as const;
