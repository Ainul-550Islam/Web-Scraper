// useDebounced — split from useDebounced.ts; barrel re-exports keep every public path & name

import { useEffect, useState } from "react";

export function useDebounced<T>(value: T, ms = 200): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = window.setTimeout(() => setV(value), ms);
    return () => window.clearTimeout(t);
  }, [value, ms]);
  return v;
}
