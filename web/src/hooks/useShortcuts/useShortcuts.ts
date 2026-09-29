// useShortcuts — split from useShortcuts.ts; barrel re-exports keep every public path & name

import { useEffect } from "react";
import { GOTO } from "./GOTO";
import { isEditable } from "./isEditable";


export function useShortcuts(enabled = true): void {
  useEffect(() => {
    if (!enabled) return;
    let armedAt = 0;
    const onKey = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (isEditable(e.target)) return;
      const now = Date.now();
      if (e.key === "g") {
        armedAt = now;
        return;
      }
      if (now - armedAt <= 1_500) {
        const to = GOTO[e.key];
        if (to !== undefined) {
          e.preventDefault();
          window.location.hash = `#${to}`;
        }
        armedAt = 0;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled]);
}
