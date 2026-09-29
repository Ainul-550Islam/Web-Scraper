// IApi — glyph for the API Explorer page (brackets + dot, same 16/1.5 grid)
import type { P } from "../P";
import { Svg } from "../Svg";

export function IApi(p: P) {
  return (
    <Svg {...p}>
      <path d="M5.5 4H4.5A1.5 1.5 0 0 0 3 5.5v3L2 12l1 3.5v3A1.5 1.5 0 0 0 5.5 20h1" />
      <path d="M18.5 4h1A1.5 1.5 0 0 1 21 5.5v3l1 3.5-1 3.5v3a1.5 1.5 0 0 1-1.5 1.5h-1" />
      <circle cx="12" cy="12" r="1.4" />
    </Svg>
  );
}
