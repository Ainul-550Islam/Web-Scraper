import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Checkbox } from "../../components/patterns";

describe("Checkbox (patterns/Checkbox.tsx)", () => {
  it("native checkbox semantics with boolean onChange", () => {
    const onChange = vi.fn();
    const { getByRole } = render(<Checkbox checked onChange={onChange} label="respect" />);
    const el = getByRole("checkbox");
    expect(el).toBeChecked();
    fireEvent.click(el);
    expect(onChange).toHaveBeenCalledWith(false);
  });
});
