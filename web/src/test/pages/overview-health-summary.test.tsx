// Overview's HealthSummary: six groups, values only from fixtures, "—" when
// a source is missing; quarantine is shown as diverted, never as dropped.

import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import { mountPage } from "../harness";
import Overview from "../../features/overview/Overview";
import stats from "../fixtures/stats.json";
import runtime from "../fixtures/runtime.json";
import storage from "../fixtures/storage.json";

describe("Overview · HealthSummary", () => {
  it("renders all six groups with fixture-backed values", async () => {
    await mountPage(<Overview />);
    const list = await screen.findByRole("list", { name: "Product health summary" });
    for (const group of ["System", "Execution", "Network", "Data", "Storage", "Security"]) {
      expect(list.textContent, group).toContain(group);
    }
    expect(document.body.textContent).toContain(`written`);
    expect(document.body.textContent).toContain("diverted (never dropped silently)");
    expect(document.body.textContent).toContain("loopback open");
    // spot-check a real number from the corpus instead of any placeholder
    const m = (stats as unknown as { metrics: Record<string, number> }).metrics;
    expect(document.body.textContent).toContain(`${m.records_written} written`);
    const rt = runtime as unknown as { queue: { depth: number } };
    expect(document.body.textContent).toContain(`${rt.queue.depth} waiting`);
    const lanes = Object.values(storage as Record<string, unknown>).length;
    expect(document.body.textContent).toContain(`lane(s) configured`);
    expect(lanes).toBeGreaterThan(0);
  });

  it("shows dashes, not zeros, before data arrives", async () => {
    // force every poll empty: overrides to null are rejected by fetch stub?
    // Use __all null → JSON "null" bodies; page must render without invented numbers.
    await mountPage(<Overview />, { __all: null });
    const list = await screen.findByRole("list", { name: "Product health summary" });
    expect(list.textContent).toContain("—");
    expect(list.textContent).not.toMatch(/0 written/);
  });
});
