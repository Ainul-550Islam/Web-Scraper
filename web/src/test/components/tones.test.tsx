import { describe, expect, it } from "vitest";
import { TONES, toneFor } from "../../components/ui";

describe("tone vocabulary (components/ui/TONES.tsx + toneFor.tsx)", () => {
  it("maps every engine status word the console renders", () => {
    for (const s of [
      "ok", "running", "healthy", "completed", "degraded", "retrying", "error",
      "failed", "unavailable", "disabled", "paused", "cancelled", "drained", "queued",
    ])
      expect(TONES[s], s).toBeTruthy();
  });
  it("unknown, null and undefined fall back to idle — never a guess", () => {
    expect(toneFor("alien-status")).toBe("idle");
    expect(toneFor(null)).toBe("idle");
    expect(toneFor(undefined)).toBe("idle");
    expect(toneFor("running")).toBe("info");
  });
});
