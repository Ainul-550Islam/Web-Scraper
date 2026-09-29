// ConfigSection — split from configSchema.ts; barrel re-exports keep every public path & name



export type ConfigSection =
  | "General"
  | "Network"
  | "Reliability"
  | "Politeness"
  | "Data"
  | "Checkpoint"
  | "Operations"
  | "Alerts";
