// setPath — split from configSchema.ts; barrel re-exports keep every public path & name



export function setPath<T>(draft: T, path: string, value: unknown): T {
  const keys = path.split(".");
  const clone = structuredClone(draft) as Record<string, unknown>;
  let cur = clone;
  for (const k of keys.slice(0, -1)) {
    if (typeof cur[k] !== "object" || cur[k] === null) cur[k] = {};
    cur = cur[k] as Record<string, unknown>;
  }
  cur[keys[keys.length - 1] as string] = value;
  return clone as T;
}
