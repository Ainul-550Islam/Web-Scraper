// Pause glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IPause(p: P) {
  return (
    <Svg {...p}>
      <path d="M9 5v14M15 5v14" />
    </Svg>
  );
}
