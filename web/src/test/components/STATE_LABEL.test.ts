import { describe, expect, it } from "vitest";
import { STATE_LABEL } from "../../components/AppShell/STATE_LABEL";
import type { PollState } from "../../hooks/usePolled";

describe("STATE_LABEL (AppShell/STATE_LABEL.tsx)", () => {
  it("covers exactly the poll states — one word each, no silent gaps", () => {
    const states: PollState[] = ["connecting", "live", "refreshing", "paused", "offline"];
    expect(Object.keys(STATE_LABEL).sort()).toEqual([...states].sort());
    for (const s of states) expect(typeof STATE_LABEL[s]).toBe("string");
  });
});
