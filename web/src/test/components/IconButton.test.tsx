import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { IconButton } from "../../components/ui";

describe("IconButton (components/ui/IconButton.tsx)", () => {
  it("label is the accessible name AND the CSS tooltip source", () => {
    const { getByRole } = render(<IconButton label="Theme">◐</IconButton>);
    const el = getByRole("button", { name: "Theme" });
    expect(el.getAttribute("data-tip")).toBe("Theme");
    expect(el.getAttribute("type")).toBe("button");
  });
});
