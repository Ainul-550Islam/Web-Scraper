// StudioIndex: control counts come from the registry itself; anchors target
// the real FormSection ids.

import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { StudioIndex } from "../../features/config/ConfigStudio/StudioIndex";
import { CONTROLS, CONTROLS_BY_SECTION } from "../../lib/configSchema";
import { ROUTES } from "../../app/routes";

describe("StudioIndex", () => {
  it("computes the headline count from CONTROLS", () => {
    const { container } = render(<StudioIndex dirtyPaths={new Set()} fieldErrors={new Map()} />);
    expect(container.textContent).toContain(`${CONTROLS.length} supported configuration controls`);
    expect(CONTROLS.length).toBeGreaterThanOrEqual(30);
    for (const { section, controls } of CONTROLS_BY_SECTION) {
      expect(container.textContent).toContain(section);
      expect(container.querySelector(`a[href="#sec-${section.toLowerCase()}"]`)).toBeTruthy();
      expect(controls.length).toBeGreaterThan(0);
    }
  });

  it("flags dirty and invalid sections", () => {
    const first = CONTROLS[0]!;
    const { container } = render(
      <StudioIndex
        dirtyPaths={new Set([first.path])}
        fieldErrors={new Map([[first.path, "bad"]])}
      />,
    );
    expect(container.textContent).toContain("1 edited");
    expect(container.textContent).toContain("1 invalid");
  });

  it("keeps ROUTES metadata usable for nav (parity guard)", () => {
    expect(ROUTES.length).toBeGreaterThanOrEqual(15);
    expect(ROUTES.every((r) => r.title.trim().length > 0 && (r.desc ?? "").trim().length > 0)).toBe(true);
  });
});
