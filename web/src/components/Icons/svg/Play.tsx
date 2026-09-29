// Play glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IPlay(p: P) {
  return (
    <Svg {...p}>
      <path d="M7 4.8v14.4L19 12 7 4.8Z" />
    </Svg>
  );
}
