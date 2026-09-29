// Jobs glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IJobs(p: P) {
  return (
    <Svg {...p}>
      <rect x="3.5" y="5.5" width="17" height="13" rx="2" />
      <path d="M8 3v3M16 3v3M3.5 10.5h17" />
    </Svg>
  );
}
