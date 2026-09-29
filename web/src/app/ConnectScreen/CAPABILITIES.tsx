// CAPABILITIES — split from ConnectScreen.tsx; barrel re-exports keep every public path & name


export const CAPABILITIES: readonly string[] = [
  "Live engine state: health, runtime, queue, workers, proxy pool",
  "Run history, jobs, activity ring, pipeline & data-quality views",
  "Sealed audit journal, alerts, diagnostics, storage observatory",
  "Read-only Configuration Studio over the real server document (37 controls)",
  "Target YAML builder for the same schema the engine validates",
];
