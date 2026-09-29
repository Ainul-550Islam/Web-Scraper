import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import App from "../../app/App";
import { mountPage } from "../harness";
import { invalidateCache } from "../../lib/api";

describe("theme (§41)", () => {
  it("persists ONLY the theme preference and applies it to the root", async () => {
    invalidateCache();
    mountPage(<App />);
    const group = await screen.findByRole("group", { name: "Theme" });
    fireEvent.click(within(group).getByRole("button", { name: "Light" }));
    expect(document.documentElement.dataset["theme"]).toBe("light");
    expect(localStorage.getItem("harvest.console.theme")).toBe("light");
    // §32: only the allow-listed non-sensitive preferences may ever persist
    const ALLOWED = new Set([
      "harvest.console.theme",
      "harvest.console.nav",
      "harvest.console.cadence",
      "harvest.console.api",
      "harvest.console.tour",
    ]);
    for (const k of Object.keys(localStorage)) expect(ALLOWED.has(k), k).toBe(true);
    const dump = Object.entries(Object.getOwnPropertyNames(localStorage)).join();
    expect(dump.toLowerCase()).not.toMatch(/token|bearer|secret|password/);
  });
});
