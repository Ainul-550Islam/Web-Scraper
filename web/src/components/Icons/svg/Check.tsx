// Check glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function ICheck(p: P) {
  return (
    <Svg {...p}>
      <path d="m4.5 12.5 5 5 10-11" />
    </Svg>
  );
}
