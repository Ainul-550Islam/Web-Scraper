import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { PipelineNode } from "../../components/patterns";

describe("PipelineNode (patterns/PipelineNode.tsx)", () => {
  it("interactive by default: button with selection state", () => {
    const onSelect = vi.fn();
    const { getByRole } = render(
      <PipelineNode title="Fetch" selected onSelect={onSelect} />,
    );
    const el = getByRole("button");
    expect(el.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(el);
    expect(onSelect).toHaveBeenCalledTimes(1);
  });
  it("as=div is the read-only strip variant", () => {
    const { queryByRole, container } = render(<PipelineNode title="Fetch" as="div" />);
    expect(queryByRole("button")).toBeNull();
    expect(container.querySelector(".pipe-node")).toBeTruthy();
  });
});
