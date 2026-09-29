import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { FilterBar } from "../../components/patterns";

describe("FilterBar (patterns/FilterBar.tsx)", () => {
  it("is a plain composition slot that merges className", () => {
    const { container } = render(<FilterBar className="wide"><i>f</i></FilterBar>);
    expect(container.querySelector(".filterbar.wide")).toBeTruthy();
  });
});
