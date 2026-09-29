import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Field } from "../../components/ui";

describe("Field (components/ui/Field.tsx)", () => {
  it("associates the label, marks dirtiness, and links help text", () => {
    const { getByText, getByLabelText, container } = render(
      <Field label="Concurrency" htmlFor="conc" help="parallel workers" dirty>
        <input id="conc" aria-label="Concurrency" />
      </Field>,
    );
    expect(container.querySelector(".ctrl")?.getAttribute("data-dirty")).toBe("true");
    expect(getByText("parallel workers").id).toBe("conc-help");
    expect(getByLabelText("Concurrency")).toBeTruthy();
  });
  it("renders the error row with role=alert", () => {
    const { getByRole } = render(
      <Field label="L" error="must be ≥ 1"><input /></Field>,
    );
    expect(getByRole("alert").textContent).toContain("must be ≥ 1");
  });
});
