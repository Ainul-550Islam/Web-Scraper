// IUsage — glyph for the Usage page (three bars on the same 16/1.5 grid as the others)
import type { P } from "../P";
import { Svg } from "../Svg";

export function IUsage(p: P) {
  return (
    <Svg {...p}>
      <rect x="3" y="10" width="3" height="6" rx="1" />
      <rect x="8.5" y="6" width="3" height="10" rx="1" />
      <rect x="14" y="3" width="3" height="13" rx="1" />
    </Svg>
  );
}
