// Operations section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const OPERATIONS_CONTROLS: ControlDef[] = [
  // ── Operations ───────────────────────────────────────────────
    { path: "metrics_addr", label: "Admin bind address", section: "Operations", kind: "text",
      help: "Admin/API listener. Non-loopback binds require an admin_token (validate enforces this)." },
  { path: "admin_token", label: "Admin token", section: "Operations", kind: "secret", secret: true,
      help: "Read-only here: secrets live in the config file / ${ENV} only. The API shows <redacted>." },
  { path: "attest_key_path", label: "Attestation key path", section: "Operations", kind: "path",
      help: "0600 HMAC key that seals audit + manifest evidence." },
  { path: "schedule.every_secs", label: "Schedule interval", section: "Operations", kind: "number", int: true, min: 1, unit: "s",
      help: "serve mode cadence between runs." },
  { path: "schedule.runs", label: "Run count limit", section: "Operations", kind: "number", int: true, min: 0,
      help: "0 = forever (serve mode)." },
];
