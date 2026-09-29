import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { ToastProvider, useToasts } from "../../hooks/useToasts";

function Board() {
  const { toasts, push, dismiss } = useToasts();
  return (
    <div>
      <output data-testid="n">{toasts.length}</output>
      <output data-testid="msgs">{toasts.map((t) => `${t.id}:${t.message}`).join(",")}</output>
      <button onClick={() => push("ok", "saved")}>ok</button>
      <button onClick={() => push("err", "boom")}>err</button>
      <button onClick={() => toasts[0] !== undefined && dismiss(toasts[0].id)}>drop-first</button>
    </div>
  );
}

describe("useToasts + ToastProvider (hooks/useToasts)", () => {
  it("caps history at five — oldest is silently evicted, ids keep counting", () => {
    const { getByRole, getByTestId } = render(
      <ToastProvider><Board /></ToastProvider>,
    );
    for (let i = 0; i < 7; i++) fireEvent.click(getByRole("button", { name: "ok" }));
    expect(getByTestId("n").textContent).toBe("5");
    expect(getByTestId("msgs").textContent).toBe("3:saved,4:saved,5:saved,6:saved,7:saved");
  });
  it("dismiss removes by id", () => {
    const { getByRole, getByTestId } = render(
      <ToastProvider><Board /></ToastProvider>,
    );
    fireEvent.click(getByRole("button", { name: "ok" }));
    fireEvent.click(getByRole("button", { name: "drop-first" }));
    expect(getByTestId("n").textContent).toBe("0");
  });
  it("err toasts linger 7000ms while ok expires at 4200ms", () => {
    vi.useFakeTimers();
    try {
      const { getByRole, getByTestId } = render(
        <ToastProvider><Board /></ToastProvider>,
      );
      fireEvent.click(getByRole("button", { name: "ok" }));
      fireEvent.click(getByRole("button", { name: "err" }));
      act(() => void vi.advanceTimersByTime(4200));
      expect(getByTestId("msgs").textContent).toBe("2:boom");
      act(() => void vi.advanceTimersByTime(2800));
      expect(getByTestId("n").textContent).toBe("0");
    } finally {
      vi.useRealTimers();
    }
  });
  it("throws loudly outside the provider — no silent no-op hook", () => {
    const spy = vi.spyOn(console, "error").mockImplementation(() => undefined);
    try {
      expect(() => render(<Board />)).toThrow(/useToasts outside ToastProvider/);
    } finally {
      spy.mockRestore();
    }
  });
});
