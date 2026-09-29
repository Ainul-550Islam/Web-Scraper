import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { DemoBanner } from "../../components/AppShell/DemoBanner";

describe("DemoBanner (AppShell/DemoBanner.tsx)", () => {
  it("silent unless demo is true (compile-time gate lives upstream)", () => {
    const { container } = render(<DemoBanner demo={false} />);
    expect(container.firstChild).toBeNull();
  });
  it("announces DEMO MODE as a live status with the honesty sentence", () => {
    const { getByRole } = render(<DemoBanner demo />);
    const el = getByRole("status");
    expect(el.textContent).toContain("DEMO MODE");
    expect(el.textContent).toContain("deterministic fixtures, not live data");
  });
});
