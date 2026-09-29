import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { MetricCard } from "../../components/ui";

describe("MetricCard (components/ui/MetricCard.tsx)", () => {
  it("renders label, value, and unit in the documented slots", () => {
    const { getByText, container } = render(
      <MetricCard label="Req/s" value={12.5} unit="r/s" tone="ok" />,
    );
    expect(getByText("Req/s")).toBeTruthy();
    const value = container.querySelector(".metric-value");
    expect(value?.textContent).toContain("12.5");
    expect(value?.querySelector("small")?.textContent).toBe("r/s");
    expect(value?.getAttribute("aria-live")).toBe("off");
  });
  it("bumps the value on change and flags live metrics", () => {
    const { container, rerender } = render(<MetricCard label="a" value={1} live />);
    expect(container.querySelector(".dot.live")).toBeTruthy();
    rerender(<MetricCard label="a" value={2} live />);
    expect(container.querySelector(".metric-value")?.className).toContain("bump");
  });
});
