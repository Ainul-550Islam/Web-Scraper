import { describe, expect, it } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { useHotkeyPalette } from "../../components/Overlays/useHotkeyPalette";

function HotkeyProbe() {
  const { open, setOpen } = useHotkeyPalette();
  return (
    <>
      <output>{open ? "open" : "closed"}</output>
      <button onClick={() => setOpen(false)}>manual-close</button>
    </>
  );
}

describe("useHotkeyPalette (Overlays/useHotkeyPalette.tsx)", () => {
  it("Ctrl+K and Cmd+K toggle; plain k does nothing", () => {
    const { getByText } = render(<HotkeyProbe />);
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    expect(getByText("open")).toBeTruthy();
    fireEvent.keyDown(window, { key: "K", metaKey: true });
    expect(getByText("closed")).toBeTruthy();
    fireEvent.keyDown(window, { key: "k" });
    expect(getByText("closed")).toBeTruthy();
  });
  it("the listener is gone after unmount (no state updates on a dead tree)", () => {
    const { unmount } = render(<HotkeyProbe />);
    unmount();
    act(() => {
      fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    });
  });
});
