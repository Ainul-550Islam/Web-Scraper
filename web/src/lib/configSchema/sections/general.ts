// General section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const GENERAL_CONTROLS: ControlDef[] = [
  // ── General ──────────────────────────────────────────────────
    { path: "name", label: "Run name", section: "General", kind: "text",
      help: "Identifies runs in history, records, and audit rows.", max: 64 },
  { path: "user_agent", label: "User agent", section: "General", kind: "text",
      help: "Honest bot identity. With robots enforced, Mozilla-style UAs are refused by validation." },
  { path: "respect_robots", label: "Enforce robots.txt", section: "General", kind: "toggle",
      help: "RFC-9309-ish parsing with caching; deny skips the URL." },
];
