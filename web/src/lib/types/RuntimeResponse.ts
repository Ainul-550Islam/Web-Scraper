// RuntimeResponse — split from types.ts; barrel re-exports keep every public path & name

import type { RuntimeSnapshot } from "./RuntimeSnapshot";

export type RuntimeResponse = RuntimeSnapshot | { error: "no run active" };
