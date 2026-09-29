// Network section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const NETWORK_CONTROLS: ControlDef[] = [
  // ── Network ──────────────────────────────────────────────────
    { path: "concurrency", label: "Concurrency", section: "Network", kind: "number", int: true, min: 1, max: 256,
      help: "In-flight requests across the run (0 is rejected)." },
  { path: "request_timeout_secs", label: "Request timeout", section: "Network", kind: "number", int: true, min: 1, unit: "s",
      help: "Per-request response timeout." },
  { path: "connect_timeout_secs", label: "Connect timeout", section: "Network", kind: "number", int: true, min: 1, unit: "s",
      help: "TCP+TLS establishment budget per attempt." },
  { path: "max_response_bytes", label: "Response size cap", section: "Network", kind: "number", int: true, min: 1, unit: "B",
      help: "Enforced twice (declared Content-Length and streamed). Oversize is permanent — never retried." },
  { path: "max_inflight_per_proxy", label: "Per-proxy inflight cap", section: "Network", kind: "number", int: true, min: 0,
      help: "0 = unlimited. Bounds simultaneous requests on one egress." },
];
