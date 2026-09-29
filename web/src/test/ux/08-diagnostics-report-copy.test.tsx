import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import Diagnostics from "../../features/diagnostics/Diagnostics";
import { mountPage } from "../harness";
import { invalidateCache } from "../../lib/api";

describe("diagnostics report copy (§27)", () => {
  it("copies a redacted structured report and confirms visually", async () => {
    const writeText = vi.fn((text: string) => { void text; return Promise.resolve(); });
    Object.defineProperty(window.navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    invalidateCache();
    mountPage(<Diagnostics />);
    const btn = await screen.findByRole("button", { name: /Copy diagnostic report/i }, { timeout: 3000 });
    fireEvent.click(btn);
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const text: string = writeText.mock.calls[0]![0] as string;
    expect(text).toMatch(/diagnostics/i);
    expect(text).not.toMatch(/Bearer [A-Za-z0-9._-]{8,}|hunter2|password=" [^"]/);
    expect(await screen.findByText("Redacted diagnostic report copied")).toBeInTheDocument();
  });
});
