import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Card } from "../../components/ui";

describe("Card (components/ui/Card.tsx)", () => {
  it("renders head/body/foot regions only when provided", () => {
    const { container } = render(<Card>plain</Card>);
    expect(container.querySelector(".card-head")).toBeNull();
    expect(container.querySelector(".card-foot")).toBeNull();
    expect(container.querySelector(".card-body")?.textContent).toBe("plain");
  });
  it("title becomes the h2.t-section and hover toggles the modifier class", () => {
    const { container, getByRole } = render(<Card title="Runs" hover>body</Card>);
    expect(getByRole("heading", { level: 2 }).textContent).toBe("Runs");
    expect(container.querySelector(".card")?.className).toContain("card--hover");
  });
});
