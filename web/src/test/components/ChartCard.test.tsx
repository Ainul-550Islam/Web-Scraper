import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { ChartCard } from "../../components/patterns";

describe("ChartCard (patterns/ChartCard.tsx)", () => {
  it("always renders the chart body region with its distinct class", () => {
    const { container, getByRole } = render(
      <ChartCard title="Latency" meta="last 200 runs"><span>svg</span></ChartCard>,
    );
    expect(getByRole("heading", { level: 2 }).textContent).toBe("Latency");
    expect(container.querySelector(".chart-body")).toBeTruthy();
    expect(container.querySelector(".card")?.className).toContain("chart-card");
  });
});
