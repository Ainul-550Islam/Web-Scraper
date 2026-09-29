import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { TextArea } from "../../components/ui";

describe("TextArea (components/ui/TextArea.tsx)", () => {
  it("defaults to 4 rows, honors overrides, and flags invalid", () => {
    const onChange = vi.fn();
    const { getByRole } = render(
      <TextArea aria-label="pattern" rows={2} invalid onChange={onChange} />,
    );
    const el = getByRole("textbox");
    expect(el.getAttribute("rows")).toBe("2");
    expect(el.className).toBe("textarea");
    expect(el.getAttribute("aria-invalid")).toBe("true");
    fireEvent.change(el, { target: { value: "h1" } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
