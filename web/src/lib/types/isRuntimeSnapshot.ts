// isRuntimeSnapshot — split from types.ts; barrel re-exports keep every public path & name

import type { RuntimeSnapshot } from "./RuntimeSnapshot";
import type { RuntimeResponse } from "./RuntimeResponse";


export function isRuntimeSnapshot(
  r: RuntimeResponse,
): r is RuntimeSnapshot {
  return !("error" in r);
}
