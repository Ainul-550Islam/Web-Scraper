// ControlKind — split from configSchema.ts; barrel re-exports keep every public path & name


export type ControlKind =
  | "text"
  | "number"
  | "float"
  | "toggle"
  | "select"
  | "path"
  | "url"
  | "secret";
