import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Button } from "../../components/ui";

describe("Button (components/ui/Button.tsx)", () => {
  it("is always type=button and carries the variant class", () => {
    const { getByRole } = render(<Button variant="danger">Kill</Button>);
    const el = getByRole("button", { name: "Kill" });
    expect(el.getAttribute("type")).toBe("button");
    expect(el.className).toContain("btn");
    expect(el.className).toContain("danger");
  });
  it("className merges onto the base btn class", () => {
    const { getByRole } = render(<Button className="x">Merge</Button>);
    const el = getByRole("button");
    expect(el.classList.contains("btn")).toBe(true);
    expect(el.classList.contains("x")).toBe(true);
  });
  it("disabled passes through", () => {
    const { getByRole } = render(<Button disabled>Off</Button>);
    expect(getByRole("button")).toBeDisabled();
  });
  it("fires onClick only when enabled", () => {
    const fn = vi.fn();
    const { getByRole } = render(<Button onClick={fn}>Run</Button>);
    fireEvent.click(getByRole("button"));
    expect(fn).toHaveBeenCalledTimes(1);
  });
});
