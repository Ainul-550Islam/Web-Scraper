// ChevronRight glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IChevronRight(p: P) {
  return (
    <Svg {...p}>
      <path d="m9 5 7 7-7 7" />
    </Svg>
  );
}
