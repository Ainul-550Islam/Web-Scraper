import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Icon, type IconName } from "../../components/Icons";

describe("icon registry (components/Icons)", () => {
  it("every registered name resolves to an <svg> glyph", () => {
    const names = Object.keys(Icon) as IconName[];
    expect(names.length).toBeGreaterThanOrEqual(28);
    for (const name of names) {
      const Glyph = Icon[name];
      const { container } = render(<Glyph />);
      expect(container.querySelector("svg"), name).toBeTruthy();
    }
  });
});
