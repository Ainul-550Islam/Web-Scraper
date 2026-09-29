// RecentFailure — split from types.ts; barrel re-exports keep every public path & name


export interface RecentFailure {
  url?: string;
  status?: number;
  error?: string;
  [k: string]: unknown;
}
