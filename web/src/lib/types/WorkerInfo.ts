// WorkerInfo — split from types.ts; barrel re-exports keep every public path & name


export interface WorkerInfo {
  id: number;
  idle_secs: number;
  jobs_done: number;
  jobs_failed: number;
  restarts: number;
  state: string;
}
