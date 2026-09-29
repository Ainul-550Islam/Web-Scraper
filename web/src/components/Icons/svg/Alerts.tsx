// Alerts glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IAlerts(p: P) {
  return (
    <Svg {...p}>
      <path d="M18 9a6 6 0 1 0-12 0c0 5-2 6.5-2 6.5h16S18 14 18 9Z" />
      <path d="M13.7 19.5a2 2 0 0 1-3.4 0" />
    </Svg>
  );
}
