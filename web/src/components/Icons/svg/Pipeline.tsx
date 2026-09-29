// Pipeline glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IPipeline(p: P) {
  return (
    <Svg {...p}>
      <rect x="2.5" y="4" width="6" height="4.5" rx="1.2" />
      <rect x="15.5" y="4" width="6" height="4.5" rx="1.2" />
      <rect x="9" y="15.5" width="6" height="4.5" rx="1.2" />
      <path d="M5.5 8.5v3a2 2 0 0 0 2 2h1M18.5 8.5v3a2 2 0 0 1-2 2h-1M12 13.5v2" />
    </Svg>
  );
}
