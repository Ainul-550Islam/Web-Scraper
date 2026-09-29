import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Tooltip } from "../../components/patterns";

describe("Tooltip (patterns/Tooltip.tsx)", () => {
  it("wraps content in a focusable, describedby-linked tooltip", () => {
    const { getByRole, getByText } = render(<Tooltip tip="copy config"><b>x</b></Tooltip>);
    const wrap = getByText("x").parentElement!;
    expect(wrap.getAttribute("tabindex")).toBe("0");
    const tip = getByRole("tooltip");
    expect(tip.textContent).toBe("copy config");
    expect(wrap.getAttribute("aria-describedby")).toBe(tip.id);
  });
});
