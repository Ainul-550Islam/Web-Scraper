// Queue glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IQueue(p: P) {
  return (
    <Svg {...p}>
      <path d="M12 3.5 21 8l-9 4.5L3 8l9-4.5Z" />
      <path d="M4.6 12 3 12.8l9 4.5 9-4.5L19.4 12M4.6 16 3 16.8l9 4.5 9-4.5L19.4 16" />
    </Svg>
  );
}
