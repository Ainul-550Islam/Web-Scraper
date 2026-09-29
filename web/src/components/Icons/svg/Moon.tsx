// Moon glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IMoon(p: P) {
  return (
    <Svg {...p}>
      <path d="M20 14.2A8.5 8.5 0 0 1 9.8 4a8.5 8.5 0 1 0 10.2 10.2Z" />
    </Svg>
  );
}
