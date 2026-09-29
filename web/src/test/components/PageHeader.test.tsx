import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { PageHeader } from "../../components/ui";

describe("PageHeader (components/ui/PageHeader.tsx)", () => {
  it("title is the h1 and actions render on the right", () => {
    const { getByRole, getByText } = render(
      <PageHeader title="Queue" sub="2 jobs" actions={<button>Flush</button>} />,
    );
    expect(getByRole("heading", { level: 1 }).textContent).toBe("Queue");
    expect(getByText("2 jobs").className).toContain("t-caption");
    expect(getByRole("button", { name: "Flush" })).toBeTruthy();
  });
});
