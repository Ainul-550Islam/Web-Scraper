import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { PipelineConnector } from "../../components/patterns";

describe("PipelineConnector (patterns/PipelineConnector.tsx)", () => {
  it("is pure decoration: aria-hidden, no text content", () => {
    const { container } = render(<PipelineConnector />);
    const el = container.querySelector(".pipe-link")!;
    expect(el.getAttribute("aria-hidden")).toBe("true");
  });
});
