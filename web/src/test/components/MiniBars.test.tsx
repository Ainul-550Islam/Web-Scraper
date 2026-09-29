import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { MiniBars } from "../../components/ui";

describe("MiniBars (components/ui/MiniBars.tsx)", () => {
  it("is an img-labelled series with one bar per sample, zeros dimmed", () => {
    const { container, getByRole } = render(<MiniBars values={[4, 0, 2]} label="latency" />);
    expect(getByRole("img").getAttribute("aria-label")).toBe("latency");
    const bars = container.querySelectorAll(".bars i");
    expect(bars.length).toBe(3);
    expect(bars[1]?.getAttribute("class")).toBe("empty");
  });
  it("falls back to the honest empty label", () => {
    const { getByRole } = render(<MiniBars values={[1]} />);
    expect(getByRole("img").getAttribute("aria-label")).toBe("recent samples");
  });
});
