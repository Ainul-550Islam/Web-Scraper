// TRANSFORM_ARGS — split from configSchema.ts; barrel re-exports keep every public path & name

import type { TransformOp } from "./TransformOp";
import type { TransformArg } from "./TransformArg";

export const TRANSFORM_ARGS: Record<TransformOp, TransformArg[]> = {
  trim: [],
  lowercase: [],
  uppercase: [],
  collapse_whitespace: [],
  regex_replace: [
    { name: "pattern", kind: "string", required: true },
    { name: "replacement", kind: "string", required: true },
  ],
  regex_extract: [
    { name: "pattern", kind: "string", required: true },
    { name: "group", kind: "number", required: false, default: 0 },
  ],
  strip_tags: [],
  mask_pii: [
    { name: "kinds", kind: "list", required: false, options: ["email", "phone", "card", "ipv4"] },
    { name: "strategy", kind: "string", required: false },
  ],
  parse_number: [{ name: "as_int", kind: "bool", required: false, default: false }],
  round_to: [{ name: "decimals", kind: "number", required: true }],
  units: [
    { name: "to", kind: "string", required: true, options: ["bytes", "seconds", "percent"] },
  ],
  parse_date: [
    { name: "output", kind: "string", required: false, default: "rfc3339", options: ["rfc3339", "unix"] },
  ],
  to_bool: [],
  split: [
    { name: "sep", kind: "string", required: true },
    { name: "trim", kind: "bool", required: false, default: false },
    { name: "max", kind: "number", required: false, default: 0 },
  ],
  join: [{ name: "sep", kind: "string", required: true }],
  unique: [],
  sort: [],
  take: [{ name: "count", kind: "number", required: true }],
  drop: [{ name: "count", kind: "number", required: true }],
  truncate: [{ name: "max_chars", kind: "number", required: true }],
  replace_empty: [{ name: "with", kind: "string", required: true }],
  prefix: [{ name: "with", kind: "string", required: true }],
  suffix: [{ name: "with", kind: "string", required: true }],
};
