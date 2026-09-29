// SECTIONS — split from configSchema.ts; barrel re-exports keep every public path & name

import type { ConfigSection } from "./ConfigSection";


export const SECTIONS: ConfigSection[] = [
  "General",
  "Network",
  "Reliability",
  "Politeness",
  "Data",
  "Checkpoint",
  "Operations",
  "Alerts",
];
