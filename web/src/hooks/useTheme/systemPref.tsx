// systemPref — split from useTheme.tsx; barrel re-exports keep every public path & name

import type { Resolved } from "./Resolved";


export function systemPref(): Resolved {
  return typeof window !== "undefined" &&
    window.matchMedia?.("(prefers-color-scheme: light)").matches
    ? "light"
    : "dark";
}
