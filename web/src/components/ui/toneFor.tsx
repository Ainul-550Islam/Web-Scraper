// toneFor — split from ui.tsx; barrel re-exports keep every public path & name

import type { Tone } from "./Tone";
import { TONES } from "./TONES";

export function toneFor(status: string | null | undefined): Tone {
  return (status && TONES[status]) || "idle";
}
