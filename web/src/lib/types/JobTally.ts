// JobTally — split from types.ts; barrel re-exports keep every public path & name



/* ── jobs (per-target tallies of the live run, else last run) ── */
export interface JobTally {
  target: string;
  written: number;
  blocked: number;
  failed: number;
  quarantined: number;
  revalidated: number;
}
