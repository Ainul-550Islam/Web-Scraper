import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Logo } from "../../components/AppShell/Logo";

describe("Logo (AppShell/Logo.tsx)", () => {
  it("is decorative art with the size override honored", () => {
    const { container } = render(<Logo size={40} />);
    const svg = container.querySelector("svg")!;
    expect(svg.getAttribute("aria-hidden")).toBe("true");
    expect(svg.getAttribute("width")).toBe("40");
    expect(svg.getAttribute("viewBox")).toBe("0 0 32 32");
  });
});
