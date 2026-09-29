import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { SelectInput } from "../../components/ui";

describe("SelectInput (components/ui/SelectInput.tsx)", () => {
  it("keeps native select semantics with class + invalid flag", () => {
    const onChange = vi.fn();
    const { getByRole } = render(
      <SelectInput aria-label="sink" invalid onChange={onChange}>
        <option value="jsonl">jsonl</option>
        <option value="pg">postgres</option>
      </SelectInput>,
    );
    const el = getByRole("combobox");
    expect(el.className).toBe("select");
    expect(el.getAttribute("aria-invalid")).toBe("true");
    fireEvent.change(el, { target: { value: "pg" } });
    expect(onChange).toHaveBeenCalledTimes(1);
  });
});
