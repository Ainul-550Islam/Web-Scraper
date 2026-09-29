// RunsQuery — split from types.ts; barrel re-exports keep every public path & name


export interface RunsQuery {
  status?: string;
  sort?: "ts" | "duration" | "written";
  limit?: number;
  offset?: number;
}
