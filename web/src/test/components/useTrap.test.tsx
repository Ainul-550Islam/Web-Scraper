import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { useTrap } from "../../components/Overlays/useTrap";

function TrapProbe({ onClose }: { onClose: () => void }) {
  const ref = useTrap(onClose);
  return (
    <div ref={ref} tabIndex={-1}>
      <button id="a">A</button>
      <button id="b">B</button>
    </div>
  );
}

describe("useTrap (Overlays/useTrap.tsx)", () => {
  it("focuses the first focusable on mount and restores focus on unmount", () => {
    const outside = document.createElement("button");
    document.body.appendChild(outside);
    outside.focus();
    const { getByRole, unmount } = render(<TrapProbe onClose={() => undefined} />);
    expect(document.activeElement).toBe(getByRole("button", { name: "A" }));
    unmount();
    expect(document.activeElement).toBe(outside);
    outside.remove();
  });
  it("Escape closes; Tab wraps last→first and Shift+Tab first→last", () => {
    const onClose = vi.fn();
    const { getByRole } = render(<TrapProbe onClose={onClose} />);
    const a = getByRole("button", { name: "A" });
    const b = getByRole("button", { name: "B" });
    b.focus();
    fireEvent.keyDown(document, { key: "Tab" });
    expect(document.activeElement).toBe(a);
    fireEvent.keyDown(document, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(b);
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
