// useRoute — split from router.ts; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";
import type { RouteState } from "./RouteState";
import { parseHash } from "./parseHash";


export function useRoute(): RouteState {
  const [r, setR] = useState<RouteState>(parseHash);
  useEffect(() => {
    const on = () => setR(parseHash());
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  return r;
}
