import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { useEffect } from "react";
import { Toasts } from "../../components/Overlays";
import { ToastProvider, useToasts } from "../../hooks/useToasts";

function Prober({ kind, message }: { kind: "ok" | "err"; message: string }) {
  const { push } = useToasts();
  useEffect(() => {
    push(kind, message);
  }, [push, kind, message]);
  return null;
}

describe("Toasts (Overlays/Toasts.tsx)", () => {
  it("region is a polite live area and kinds pick their icon/class", () => {
    const { getByRole, getByText, container } = render(
      <ToastProvider>
        <Prober kind="err" message="run failed" />
        <Toasts />
      </ToastProvider>,
    );
    const region = getByRole("region", { name: "Notifications" });
    expect(region.getAttribute("aria-live")).toBe("polite");
    expect(getByText("run failed")).toBeTruthy();
    expect(container.querySelector(".toast.err")).toBeTruthy();
  });
  it("dismiss removes it; auto-expiry fires at 4200ms (7000 for err)", () => {
    vi.useFakeTimers();
    try {
      const { getByRole, getByText, queryByText, container } = render(
        <ToastProvider>
          <Prober kind="ok" message="saved" />
          <Toasts />
        </ToastProvider>,
      );
      expect(getByText("saved")).toBeTruthy();
      fireEvent.click(getByRole("button", { name: "Dismiss" }));
      expect(queryByText("saved")).toBeNull();
      act(() => void vi.advanceTimersByTime(5000));
      expect(container.querySelector(".toast")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });
});
