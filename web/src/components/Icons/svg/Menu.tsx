// Menu glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IMenu(p: P) {
  return (
    <Svg {...p}>
      <path d="M4 6.5h16M4 12h16M4 17.5h16" />
    </Svg>
  );
}
