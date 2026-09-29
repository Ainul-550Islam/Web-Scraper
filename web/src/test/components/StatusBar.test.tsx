import { describe, expect, it } from "vitest";
import { StatusBar } from "../../components/AppShell/StatusBar";
import { mountPage } from "../harness";

describe("StatusBar (AppShell/StatusBar.tsx)", () => {
  it("reports connection state, API base, and engine running against real fixtures", async () => {
    const { findByRole, findByText } = await mountPage(
      <StatusBar route={{ path: "/runs", query: new URLSearchParams() }} />,
    );
    const bar = await findByRole("contentinfo");
    expect(bar.getAttribute("aria-label")).toBe("Connection status");
    expect(bar.textContent).toContain("engine ");
    await findByText(/API/);
    // after the health fixture lands the dot label reads Live (or Connecting first)
    expect(bar.textContent).toMatch(/Live|Connecting/);
  });
});
