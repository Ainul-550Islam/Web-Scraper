// CONTROLS — split from configSchema.ts; barrel re-exports keep every public path & name

import type { ControlDef } from "./ControlDef";

/** The 37 real controls exposed by the Studio. */
// assembly point: concatenates the per-section registries (order mirrors SECTIONS).

import { GENERAL_CONTROLS } from "./sections/general";
import { NETWORK_CONTROLS } from "./sections/network";
import { RELIABILITY_CONTROLS } from "./sections/reliability";
import { POLITENESS_CONTROLS } from "./sections/politeness";
import { DATA_CONTROLS } from "./sections/data";
import { CHECKPOINT_CONTROLS } from "./sections/checkpoint";
import { OPERATIONS_CONTROLS } from "./sections/operations";
import { ALERTS_CONTROLS } from "./sections/alerts";

export const CONTROLS: ControlDef[] = [
  ...GENERAL_CONTROLS,
  ...NETWORK_CONTROLS,
  ...RELIABILITY_CONTROLS,
  ...POLITENESS_CONTROLS,
  ...DATA_CONTROLS,
  ...CHECKPOINT_CONTROLS,
  ...OPERATIONS_CONTROLS,
  ...ALERTS_CONTROLS,
];
