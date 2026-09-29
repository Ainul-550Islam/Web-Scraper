// Config glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IConfig(p: P) {
  return (
    <Svg {...p}>
      <path d="M4 6h10M18 6h2M4 12h2M10 12h10M4 18h8M16 18h4" />
      <circle cx="16" cy="6" r="2" />
      <circle cx="8" cy="12" r="2" />
      <circle cx="14" cy="18" r="2" />
    </Svg>
  );
}
