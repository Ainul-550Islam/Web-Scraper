// Alerts section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const ALERTS_CONTROLS: ControlDef[] = [
  // ── Alerts ───────────────────────────────────────────────────
    { path: "alerts.cooldown_secs", label: "Alert cooldown", section: "Alerts", kind: "number", int: true, min: 0, unit: "s",
      help: "Per-alert-key suppression window." },
  { path: "alerts.min_records", label: "SLO: min records", section: "Alerts", kind: "number", int: true, min: 0,
      help: "A run below this fires a slo_alert." },
  { path: "alerts.webhook_url", label: "Alert webhook", section: "Alerts", kind: "secret", secret: true,
      help: "Typically `${ALERT_WEBHOOK}`; never stored or echoed by the console." },
];
