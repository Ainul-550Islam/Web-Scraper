// brief — split from Overview.tsx; barrel re-exports keep every public path & name

import { pretty } from "./pretty";

export function brief(d: Record<string, unknown>): string {
  return Object.entries(d)
    .slice(0, 3)
    .map(([k, v]) => `${pretty(k)} ${String(v)}`)
    .join(" · ");
}
