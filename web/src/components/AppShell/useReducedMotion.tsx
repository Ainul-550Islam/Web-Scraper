// useReducedMotion — split from AppShell.tsx; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";


export function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState(
    () => typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches,
  );
  useEffect(() => {
    const mq = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    const on = () => setReduced(mq?.matches ?? false);
    mq?.addEventListener?.("change", on);
    return () => mq?.removeEventListener?.("change", on);
  }, []);
  return reduced;
}
