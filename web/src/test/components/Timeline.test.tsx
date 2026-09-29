import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Timeline } from "../../components/patterns";

describe("Timeline (patterns/Timeline.tsx)", () => {
  it("ordered list items with per-event tone dots", () => {
    const { getAllByRole, getByRole } = render(
      <Timeline
        label="job lifecycle"
        items={[
          { title: "queued", tone: "info" },
          { title: "failed", tone: "err" },
        ]}
      />,
    );
    expect(getByRole("list").getAttribute("aria-label")).toBe("job lifecycle");
    const items = getAllByRole("listitem");
    expect(items.length).toBe(2);
    expect(items[1]?.textContent).toContain("failed");
  });
  it("defaults the label when none is given", () => {
    const { getByRole } = render(<Timeline items={[]} />);
    expect(getByRole("list").getAttribute("aria-label")).toBe("timeline");
  });
});
