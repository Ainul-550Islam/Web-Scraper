import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { FormSection } from "../../components/patterns";

describe("FormSection (patterns/FormSection.tsx)", () => {
  it("collapses via native details with an id anchor", () => {
    const { container } = render(
      <FormSection id="net" title="Network" count={3}>body</FormSection>,
    );
    const d = container.querySelector("details") as HTMLDetailsElement;
    expect(d.id).toBe("sec-net");
    expect(d.open).toBe(true);
    expect(d.querySelector("summary")?.textContent).toContain("3 control(s)");
  });
  it("warning renders as a role=note, defaultOpen=false starts collapsed", () => {
    const { container, getByRole } = render(
      <FormSection id="x" title="X" warning="careful" defaultOpen={false}>b</FormSection>,
    );
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(false);
    expect(getByRole("note").textContent).toContain("careful");
  });
});
