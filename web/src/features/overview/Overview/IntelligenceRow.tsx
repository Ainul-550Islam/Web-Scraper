// IntelligenceRow — split from Overview.tsx; barrel re-exports keep every public path & name



export interface IntelligenceRow {
  label: string;
  tone: "ok" | "warn" | "err" | "info" | "idle";
  lines: string[];
}
