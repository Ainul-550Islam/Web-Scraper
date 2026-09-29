import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Tabs } from "../../components/patterns";

describe("Tabs (patterns/Tabs.tsx)", () => {
  it("proper tablist roles with a single selected tab", () => {
    const onChange = vi.fn();
    const { getAllByRole, getByRole } = render(
      <Tabs
        ariaLabel="views"
        value="a"
        onChange={onChange}
        options={[{ value: "a", label: "A" }, { value: "b", label: "B" }]}
      />,
    );
    expect(getByRole("tablist").getAttribute("aria-label")).toBe("views");
    const [a, b] = getAllByRole("tab");
    expect(a?.getAttribute("aria-selected")).toBe("true");
    expect(b?.getAttribute("aria-selected")).toBe("false");
    fireEvent.click(b!);
    expect(onChange).toHaveBeenCalledWith("b");
  });
});
