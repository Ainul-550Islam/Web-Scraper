// getPath — split from configSchema.ts; barrel re-exports keep every public path & name



/* ── access helpers ───────────────────────────────────────────── */
export function getPath(obj: unknown, path: string): unknown {
  return path
    .split(".")
    .reduce<unknown>((acc, k) => (acc as never)?.[k as never], obj);
}
