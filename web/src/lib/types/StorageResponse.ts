// StorageResponse — split from types.ts; barrel re-exports keep every public path & name

import type { StorageLane } from "./StorageLane";

export type StorageResponse = Record<string, StorageLane>;
