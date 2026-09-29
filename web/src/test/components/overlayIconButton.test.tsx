import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { IconButton } from "../../components/Overlays";

describe("overlay IconButton (Overlays/IconButton.tsx)", () => {
  it("is the lean dialog variant: label + onClick + style passthrough", () => {
    const onClick = vi.fn();
    const { getByRole } = render(
      <IconButton label="Close dialog" onClick={onClick} style={{ marginLeft: "auto" }}>×</IconButton>,
    );
    const el = getByRole("button", { name: "Close dialog" });
    expect(el.getAttribute("type")).toBe("button");
    expect(el.getAttribute("style")).toContain("margin-left");
    fireEvent.click(el);
    expect(onClick).toHaveBeenCalledTimes(1);
  });
});
