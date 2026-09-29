import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Drawer } from "../../components/Overlays";

describe("Drawer (Overlays/Drawer.tsx)", () => {
  it("aside dialog labelled 'details' with body region", () => {
    const { getByRole, container } = render(
      <Drawer title="run #3" onClose={() => undefined}>rows</Drawer>,
    );
    const dlg = getByRole("dialog");
    expect(dlg.tagName).toBe("ASIDE");
    expect(dlg.getAttribute("aria-label")).toBe("details");
    expect(container.querySelector(".drawer-body")?.textContent).toBe("rows");
  });
  it("scrim and 'Close panel' button both close", () => {
    const onClose = vi.fn();
    const { getByRole, container } = render(<Drawer title="t" onClose={onClose}>b</Drawer>);
    fireEvent.click(container.querySelector(".scrim")!);
    fireEvent.click(getByRole("button", { name: "Close panel" }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });
});
