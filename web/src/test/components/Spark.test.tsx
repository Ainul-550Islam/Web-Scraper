import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Spark } from "../../components/ui";

describe("Spark (components/ui/Spark.tsx)", () => {
  it("refuses to draw for fewer than two samples", () => {
    const { container } = render(<Spark values={[1]} />);
    expect(container.firstChild).toBeNull();
  });
  it("labels itself honestly with the sample count by default", () => {
    const { getByRole } = render(<Spark values={[1, 3, 2]} />);
    expect(getByRole("img").getAttribute("aria-label")).toBe("trend, 3 samples");
  });
  it("draws area + line paths and marks the last point", () => {
    const { container } = render(<Spark values={[5, 1, 9]} label="latency" />);
    expect(container.querySelector("path.area")).toBeTruthy();
    expect(container.querySelector("path.line")).toBeTruthy();
    expect(container.querySelector("circle.last")).toBeTruthy();
  });
});
