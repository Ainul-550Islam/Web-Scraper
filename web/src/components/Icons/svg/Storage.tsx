// Storage glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IStorage(p: P) {
  return (
    <Svg {...p}>
      <ellipse cx="12" cy="5.8" rx="8" ry="2.8" />
      <path d="M4 5.8v12.4c0 1.55 3.58 2.8 8 2.8s8-1.25 8-2.8V5.8M4 12c0 1.55 3.58 2.8 8 2.8s8-1.25 8-2.8" />
    </Svg>
  );
}
