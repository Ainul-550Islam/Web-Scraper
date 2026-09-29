import { describe, expect, it } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import Runs from "../../features/runs/Runs";
import { mountPage } from "../harness";
import { seededRuns } from "./helpers";

describe("dialogs (§36, §33)", () => {
  it("drawer traps focus and Escape closes it", async () => {
    const { container } = await mountPage(<Runs />, { "/runs": seededRuns(3) });
    await waitFor(() => expect(container.querySelectorAll("tbody tr").length).toBeGreaterThan(0));
    fireEvent.click(container.querySelector("tbody tr")!);
    const dialog = await screen.findByRole("dialog");
    expect(dialog.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(dialog, { key: "Escape" });
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });
});
