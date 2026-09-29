// Close glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IClose(p: P) {
  return (
    <Svg {...p}>
      <path d="M5.5 5.5l13 13M18.5 5.5l-13 13" />
    </Svg>
  );
}
