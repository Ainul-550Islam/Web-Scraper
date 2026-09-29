// validateControl — split from configSchema.ts; barrel re-exports keep every public path & name

import type { ControlDef } from "./ControlDef";


/* ── local validation (mirrors the rules validate() enforces) ── */
export function validateControl(def: ControlDef, value: unknown): string | null {
  if (def.secret) return null; // never edited client-side
  switch (def.kind) {
    case "number":
    case "float": {
      const n = Number(value);
      if (value === "" || value === null || !Number.isFinite(n))
        return "must be a number";
      if (def.int && !Number.isInteger(n)) return "must be a whole number";
      if (def.min !== undefined && n < def.min)
        return `must be ≥ ${def.min}${def.unit ? " " + def.unit : ""}`;
      if (def.max !== undefined && n > def.max)
        return `must be ≤ ${def.max}${def.unit ? " " + def.unit : ""}`;
      return null;
    }
    case "text":
    case "path":
    case "url": {
      const s = String(value ?? "");
      if (def.kind !== "text" && s && s !== "${" && !s.includes("${")) {
        const ok =
          def.kind === "url"
            ? /^https?:\/\//.test(s)
            : true; // paths may be relative — engine accepts both
        if (!ok) return def.kind === "url" ? "must start with http(s)://" : null;
      }
      if (def.max !== undefined && s.length > def.max)
        return `too long (max ${def.max})`;
      return null;
    }
    case "select": {
      const s = String(value ?? "");
      if (def.options && !def.options.includes(s))
        return `must be one of: ${def.options.join(", ")}`;
      return null;
    }
    case "toggle": {
      return typeof value === "boolean" ? null : "must be true/false";
    }
    default:
      return null;
  }
}
