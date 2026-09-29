// CONTROLS_BY_SECTION — split from configSchema.ts; barrel re-exports keep every public path & name

import { SECTIONS } from "./SECTIONS";
import { CONTROLS } from "./CONTROLS";


export const CONTROLS_BY_SECTION = SECTIONS.map((s) => ({
  section: s,
  controls: CONTROLS.filter((c) => c.section === s),
}));
