// Workers glyph — one file per icon; the Icon registry assembles them

import type { P } from "../P";
import { Svg } from "../Svg";

export function IWorkers(p: P) {
  return (
    <Svg {...p}>
      <rect x="4.5" y="4.5" width="15" height="15" rx="2" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
      <path d="M9 1.8v2.7M15 1.8v2.7M9 19.5v2.7M15 19.5v2.7M1.8 9h2.7M1.8 15h2.7M19.5 9h2.7M19.5 15h2.7" />
    </Svg>
  );
}
