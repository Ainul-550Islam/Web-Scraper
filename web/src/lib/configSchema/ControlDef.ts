// ControlDef — split from configSchema.ts; barrel re-exports keep every public path & name

import type { ControlKind } from "./ControlKind";
import type { ConfigSection } from "./ConfigSection";


export interface ControlDef {
  /** dotted path into the config document, e.g. "default_policy.burst" */
  path: string;
  label: string;
  section: ConfigSection;
  kind: ControlKind;
  help: string;
  unit?: string;
  min?: number;
  max?: number;
  int?: boolean;
  options?: string[];
  /** values are never fetched/shown client-side; the draft keeps a ${VAR} ref */
  secret?: boolean;
  /** true when this control only edits the local draft (server is read-only) */
  appliedVia?: "draft";
}
