// Download glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IDownload(p: P) {
  return (
    <Svg {...p}>
      <path d="M12 3.5V15m0 0 4.5-4.5M12 15l-4.5-4.5M4 19.5h16" />
    </Svg>
  );
}
