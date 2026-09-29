import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Toggle } from "../../components/ui";

describe("Toggle (components/ui/Toggle.tsx)", () => {
  it("is a real switch: role, aria-checked, and inverted-on-click semantics", () => {
    const onChange = vi.fn();
    const { getByRole } = render(<Toggle checked={false} onChange={onChange} label="robots" />);
    const el = getByRole("switch");
    expect(el.getAttribute("aria-checked")).toBe("false");
    expect(el.getAttribute("aria-label")).toBe("robots");
    fireEvent.click(el);
    expect(onChange).toHaveBeenCalledWith(true);
  });
  it("disabled toggles never fire", () => {
    const onChange = vi.fn();
    const { getByRole } = render(
      <Toggle checked onChange={onChange} label="locked" disabled />,
    );
    fireEvent.click(getByRole("switch"));
    expect(onChange).not.toHaveBeenCalled();
  });
});
