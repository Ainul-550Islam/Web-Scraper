// Refresh glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IRefresh(p: P) {
  return (
    <Svg {...p}>
      <path d="M20 11.5a8 8 0 1 0-1.7 6.2" />
      <path d="M20 4.5V11h-6" />
    </Svg>
  );
}
