import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { StatusChip } from "../../components/ui";

describe("StatusChip (components/ui/StatusChip.tsx)", () => {
  it("uses the shared toneFor mapping and can swap the label only", () => {
    const { container, getByText } = render(<StatusChip status="failed" label="EXIT 1" />);
    expect(container.querySelector(".chip")?.className).toContain("err");
    expect(getByText("EXIT 1")).toBeTruthy();
  });
  it("dot=false removes the decoration dot", () => {
    const { container } = render(<StatusChip status="ok" dot={false} />);
    expect(container.querySelector("i.dot")).toBeNull();
  });
});
