import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { InlineError } from "../../components/ui";
import { ApiError } from "../../lib/api";

describe("InlineError (components/ui/InlineError.tsx)", () => {
  it("renders nothing for a null error", () => {
    const { container } = render(<InlineError error={null} />);
    expect(container.firstChild).toBeNull();
  });
  it("shows message plus requestId, role=status, never stacks", () => {
    const err = new ApiError("timeout", "upstream silent", { requestId: "rq-42" });
    const { getByRole } = render(<InlineError error={err} />);
    const el = getByRole("status");
    expect(el.textContent).toContain("upstream silent");
    expect(el.textContent).toContain("rq-42");
  });
});
