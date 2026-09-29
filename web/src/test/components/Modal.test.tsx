import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Modal } from "../../components/Overlays";

describe("Modal (Overlays/Modal.tsx)", () => {
  it("is a labelled modal dialog with a scrim that closes", () => {
    const onClose = vi.fn();
    const { getByRole, container } = render(
      <Modal title="Kill run" onClose={onClose}>body</Modal>,
    );
    const dlg = getByRole("dialog");
    expect(dlg.getAttribute("aria-modal")).toBe("true");
    expect(dlg.getAttribute("aria-labelledby")).toBe("modal-title");
    expect(container.querySelector(".scrim")).toBeTruthy();
    fireEvent.click(container.querySelector(".scrim")!);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("close IconButton uses its accessible name and fires onClose", () => {
    const onClose = vi.fn();
    const { getByRole } = render(<Modal title="t" onClose={onClose}>b</Modal>);
    fireEvent.click(getByRole("button", { name: "Close dialog" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("footer slot only renders when provided", () => {
    const { container } = render(
      <Modal title="t" onClose={() => undefined} footer={<button>Do</button>}>b</Modal>,
    );
    expect(container.querySelector(".card-foot")).toBeTruthy();
  });
});
