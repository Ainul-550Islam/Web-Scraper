// stageTone — split from Pipeline.tsx; barrel re-exports keep every public path & name

import type { Ctx } from "./Ctx";


export function stageTone(id: string, c: Ctx): string {
  switch (id) {
    case "schema":
      return (c.stages["schema_drifts"] ?? 0) > 0 ? "warn" : "ok";
    case "sink":
      return (c.pipe?.pipeline_failures ?? 0) > 0 ? "warn" : "ok";
    case "proxy":
      return c.proxies > 0 ? "ok" : "info";
    default:
      return "ok";
  }
}
