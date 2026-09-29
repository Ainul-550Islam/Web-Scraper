// Audit glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IAudit(p: P) {
  return (
    <Svg {...p}>
      <path d="M12 3 4.5 5.8V11c0 5 3.2 8.3 7.5 10 4.3-1.7 7.5-5 7.5-10V5.8L12 3Z" />
      <path d="m8.8 11.6 2.2 2.2 4.4-4.6" />
    </Svg>
  );
}
