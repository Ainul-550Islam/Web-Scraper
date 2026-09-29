import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { HealthCell } from "../../components/ui";

describe("HealthCell (components/ui/HealthCell.tsx)", () => {
  it("maps the four health states to their own classes, fallback err", () => {
    const cls = (status: string) =>
      render(<HealthCell name="n" status={status} />).container.querySelector(".health-cell")
        ?.className;
    expect(cls("healthy")).toContain("ok");
    expect(cls("disabled")).toContain("disabled");
    expect(cls("degraded")).toContain("degraded");
    expect(cls("on fire")).toContain("err");
  });
  it("detail overrides the status line when given", () => {
    const { getByText } = render(
      <HealthCell name="sink" status="degraded" detail="3 quarantined" />,
    );
    expect(getByText("3 quarantined")).toBeTruthy();
  });
});
