import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Segmented } from "../../components/ui";

describe("Segmented (components/ui/Segmented.tsx)", () => {
  it("exactly one option is pressed and clicks report the value", () => {
    const onChange = vi.fn();
    const { getByRole, getAllByRole } = render(
      <Segmented
        ariaLabel="range"
        value="day"
        onChange={onChange}
        options={[{ value: "day", label: "Day" }, { value: "week", label: "Week" }]}
      />,
    );
    expect(getByRole("group").getAttribute("aria-label")).toBe("range");
    const [day, week] = getAllByRole("button");
    expect(day?.getAttribute("aria-pressed")).toBe("true");
    expect(week?.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(week!);
    expect(onChange).toHaveBeenCalledWith("week");
  });
});
