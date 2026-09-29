// Overview glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IOverview(p: P) {
  return (
    <Svg {...p}>
      <rect x="3.5" y="3.5" width="7" height="9" rx="1.5" />
      <rect x="13.5" y="3.5" width="7" height="5" rx="1.5" />
      <rect x="3.5" y="15.5" width="7" height="5" rx="1.5" />
      <rect x="13.5" y="11.5" width="7" height="9" rx="1.5" />
    </Svg>
  );
}
