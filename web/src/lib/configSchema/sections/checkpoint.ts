// Checkpoint section controls — moved verbatim from the CONTROLS registry

import type { ControlDef } from "../ControlDef";

export const CHECKPOINT_CONTROLS: ControlDef[] = [
  // ── Checkpoint ───────────────────────────────────────────────
    { path: "ckpt_path", label: "Checkpoint path", section: "Checkpoint", kind: "path",
      help: "Atomic temp+rename snapshot file; `--resume` continues from it." },
  { path: "ckpt_every_secs", label: "Checkpoint interval", section: "Checkpoint", kind: "number", int: true, min: 1, unit: "s",
      help: "Time cadence — bounds the crash-loss window." },
  { path: "ckpt_every_pages", label: "Checkpoint page gate", section: "Checkpoint", kind: "number", int: true, min: 1,
      help: "Save every N records, whichever comes first with the time cadence." },
];
