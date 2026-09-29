// Data section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";
import { OUTPUT_KINDS } from "../OUTPUT_KINDS";

export const DATA_CONTROLS: ControlDef[] = [
  // ── Data / output ────────────────────────────────────────────
    { path: "output.kind", label: "Output backend", section: "Data", kind: "select", options: [...OUTPUT_KINDS],
      help: "Sink kind. Backend availability is shown in _backends." },
  { path: "output.path", label: "Output path", section: "Data", kind: "path",
      help: "File path for jsonl/csv sinks (shown when applicable)." },
  { path: "audit_path", label: "Audit log path", section: "Data", kind: "path",
      help: "Append-only attestation journal (HMAC-sealed when attesting)." },
  { path: "revalidate_store", label: "Revalidation store", section: "Data", kind: "path",
      help: "ETag/Last-Modified store so repeat crawls pay 304s, not bodies." },
  { path: "revalidate_ttl_secs", label: "Revalidation TTL", section: "Data", kind: "number", int: true, min: 0, unit: "s",
      help: "Age after which a validator is refreshed with a full GET." },
];
