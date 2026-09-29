// Warn glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IWarn(p: P) {
  return (
    <Svg {...p}>
      <path d="M12 3.5 21.5 20H2.5L12 3.5Z" />
      <path d="M12 10v4.5M12 17.5v.01" />
    </Svg>
  );
}
