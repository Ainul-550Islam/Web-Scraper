import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { NumberInput } from "../../components/ui";

describe("NumberInput (components/ui/NumberInput.tsx)", () => {
  it("null is the empty string; blank edit reports null, digits report numbers", () => {
    const onValue = vi.fn();
    const { container } = render(
      <NumberInput value={null} onValue={onValue} aria-label="cap" unit="urls" />,
    );
    const input = container.querySelector("input")!;
    expect(input.value).toBe("");
    fireEvent.change(input, { target: { value: "12" } });
    expect(onValue).toHaveBeenLastCalledWith(12);
    expect(container.querySelector(".unit")?.getAttribute("data-unit")).toBe("urls");
  });
  it("a non-empty controlled value renders the number itself", () => {
    const onValue = vi.fn();
    const { container } = render(<NumberInput value={7} onValue={onValue} aria-label="cap" />);
    fireEvent.change(container.querySelector("input")!, { target: { value: "" } });
    expect(onValue).toHaveBeenCalledWith(null);
    expect((container.querySelector("input") as HTMLInputElement).value).toBe("7");
  });
  it("invalid sets aria-invalid, unit uses decimal keypad", () => {
    const { container } = render(
      <NumberInput value={0} onValue={() => undefined} invalid aria-label="x" />,
    );
    const input = container.querySelector("input")!;
    expect(input.getAttribute("aria-invalid")).toBe("true");
    expect(input.getAttribute("inputMode")).toBe("decimal");
  });
});
