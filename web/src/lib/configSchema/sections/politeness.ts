// Politeness section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const POLITENESS_CONTROLS: ControlDef[] = [
  // ── Politeness ───────────────────────────────────────────────
    { path: "default_policy.requests_per_sec", label: "Requests / sec", section: "Politeness", kind: "float", min: 0.01,
      help: "Token-bucket rate per domain." },
  { path: "default_policy.burst", label: "Burst", section: "Politeness", kind: "number", int: true, min: 1,
      help: "Token-bucket burst allowance." },
  { path: "default_policy.min_interval_ms", label: "Min interval", section: "Politeness", kind: "number", int: true, min: 0, unit: "ms",
      help: "Minimum spacing between requests to one host." },
  { path: "scheduling.retry_budget_per_min", label: "Retry budget / min", section: "Politeness", kind: "number", int: true, min: 0,
      help: "Retries permitted per minute before budget denial." },
];
