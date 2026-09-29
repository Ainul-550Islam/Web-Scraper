// parseHash — split from router.ts; barrel re-exports keep every public path & name

import type { RouteState } from "./RouteState";


export function parseHash(): RouteState {
  const h = window.location.hash.replace(/^#/, "") || "/";
  const [path, qs] = h.split("?");
  return { path: path || "/", query: new URLSearchParams(qs ?? "") };
}
