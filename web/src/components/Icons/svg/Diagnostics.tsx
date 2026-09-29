// Diagnostics glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IDiagnostics(p: P) {
  return (
    <Svg {...p}>
      <path d="M2.5 12h4l2-6 4 12 2.5-8 1.5 2h5" />
    </Svg>
  );
}
