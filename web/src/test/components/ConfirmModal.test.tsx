import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, waitFor } from "@testing-library/react";
import { ConfirmModal } from "../../components/Overlays";

describe("ConfirmModal (Overlays/ConfirmModal.tsx)", () => {
  it("cancel never confirms", () => {
    const onConfirm = vi.fn();
    const onClose = vi.fn();
    const { getByRole } = render(
      <ConfirmModal title="t" body="b" confirmLabel="Flush" onConfirm={onConfirm} onClose={onClose} />,
    );
    fireEvent.click(getByRole("button", { name: "Cancel" }));
    expect(onConfirm).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
  it("confirm awaits the handler, shows Working…, then closes", async () => {
    let release!: () => void;
    const onConfirm = vi.fn(() => new Promise<void>((r) => (release = r)));
    const onClose = vi.fn();
    const { getByRole } = render(
      <ConfirmModal title="t" body="b" confirmLabel="Flush" danger onConfirm={onConfirm} onClose={onClose} />,
    );
    const ok = getByRole("button", { name: "Flush" });
    expect(ok.className).toContain("danger");
    fireEvent.click(ok);
    expect(getByRole("button", { name: "Working…" })).toBeDisabled();
    release();
    await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });
});
