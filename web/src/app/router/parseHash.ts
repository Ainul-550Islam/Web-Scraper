// parseHash — split from router.ts; barrel re-exports keep every public path & name

import type { RouteState } from "./RouteState";
import { ROUTE_BY_PATH } from "../routes/ROUTE_BY_PATH";


export function parseHash(): RouteState {
  const h = window.location.hash.replace(/^#/, "") || "/";
  const separator = h.indexOf("?");
  const candidate = separator < 0 ? h : h.slice(0, separator);
  const known = ROUTE_BY_PATH.has(candidate);
  const path = known ? candidate : "/";
  const query = known && separator >= 0 ? h.slice(separator + 1) : "";
  return { path, query: new URLSearchParams(query) };
}
