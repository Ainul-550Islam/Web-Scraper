// Paged — split from types.ts; barrel re-exports keep every public path & name


export interface Paged<T> {
  items: T[];
  limit: number;
  offset: number;
  total: number;
}
