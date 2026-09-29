// Plug glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IPlug(p: P) {
  return (
    <Svg {...p}>
      <path d="M8.5 3v5M15.5 3v5M5.5 8h13v3a6.5 6.5 0 0 1-13 0V8ZM12 17.5V21" />
    </Svg>
  );
}
