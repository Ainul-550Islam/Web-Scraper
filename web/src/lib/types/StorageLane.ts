// StorageLane — split from types.ts; barrel re-exports keep every public path & name



export interface StorageLane {
  kind: string;
  path: string;
  bytes: number | null;
  configured: boolean;
  exists: boolean;
}
