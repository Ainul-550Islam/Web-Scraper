import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { EmptyState } from "../../components/ui";

describe("EmptyState (components/ui/EmptyState.tsx)", () => {
  it("shows the title as h3 and a default info glyph", () => {
    const { getByRole, container } = render(<EmptyState title="No runs yet" />);
    expect(getByRole("heading", { level: 3 }).textContent).toBe("No runs yet");
    expect(container.querySelector(".state-ico svg")).toBeTruthy();
    expect(container.querySelector("p")).toBeNull();
  });
  it("renders optional body and action slots", () => {
    const { container } = render(
      <EmptyState title="t" body="why" action={<button>Start</button>} />,
    );
    expect(container.querySelector("p")?.textContent).toBe("why");
    expect(container.querySelectorAll("button").length).toBe(1);
  });
});
