import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Skeleton } from "../../components/ui";

describe("Skeleton (components/ui/Skeleton.tsx)", () => {
  it("is decoration, not content: aria-hidden with sized box", () => {
    const { container } = render(<Skeleton h={22} w="40%" />);
    const el = container.querySelector(".skeleton")!;
    expect(el.getAttribute("aria-hidden")).toBe("true");
    expect(el.getAttribute("style")).toContain("height: 22px");
    expect(el.getAttribute("style")).toContain("width: 40%");
  });
});
