// Copy glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function ICopy(p: P) {
  return (
    <Svg {...p}>
      <rect x="8.5" y="8.5" width="11" height="11" rx="2" />
      <path d="M4.5 15.5h-1v-11h11v1" />
    </Svg>
  );
}
