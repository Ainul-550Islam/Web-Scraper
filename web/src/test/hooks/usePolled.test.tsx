import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, waitFor } from "@testing-library/react";
import { usePolled } from "../../hooks/usePolled";
import { ApiError } from "../../lib/api";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function Probe({
  fetcher,
  interval = 1500,
}: {
  fetcher: (s: AbortSignal) => Promise<unknown>;
  interval?: number;
}) {
  const p = usePolled(fetcher, interval);
  return (
    <div>
      <output data-testid="state">{p.state}</output>
      <output data-testid="err">{p.error?.kind ?? "none"}</output>
      <output data-testid="data">{JSON.stringify(p.data)}</output>
      <button onClick={() => p.setPaused(true)}>pause</button>
    </div>
  );
}

describe("usePolled (hooks/usePolled)", () => {
  it("fetches on mount and reaches live with data", async () => {
    const fetcher = vi.fn(async () => ({ ok: 1 }));
    const { getByTestId } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(getByTestId("state").textContent).toBe("live"));
    expect(fetcher).toHaveBeenCalledTimes(1);
    expect(getByTestId("data").textContent).toBe('{"ok":1}');
  });
  it("interval floor: a 1000ms request is clamped to ≥1500ms", async () => {
    const fetcher = vi.fn(async () => ({}));
    render(<Probe fetcher={fetcher} interval={1000} />);
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    await sleep(1200);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(2), { timeout: 1500 });
  });
  it("network failure → offline; recovery → live with the error cleared", async () => {
    let mode = "down";
    const fetcher = vi.fn(async () => {
      if (mode === "down") throw new ApiError("network", "admin API unreachable");
      return { ok: 2 };
    });
    const { getByTestId } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(getByTestId("state").textContent).toBe("offline"));
    expect(getByTestId("err").textContent).toBe("network");
    mode = "up";
    await waitFor(() => expect(getByTestId("state").textContent).toBe("live"), { timeout: 4000 });
    expect(getByTestId("err").textContent).toBe("none");
  });
  it("429 slowdown buys a ≥3s backoff before the next attempt", async () => {
    let mode = "slow";
    const fetcher = vi.fn(async () => {
      if (mode === "slow") throw new ApiError("slowdown", "429", { status: 429 });
      return { ok: 3 };
    });
    const { getByTestId } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(getByTestId("err").textContent).toBe("slowdown"));
    const after = fetcher.mock.calls.length;
    await sleep(1300);
    expect(fetcher.mock.calls.length).toBe(after); // still inside the cooldown
    mode = "up";
    await waitFor(() => expect(fetcher.mock.calls.length).toBeGreaterThan(after), { timeout: 4000 });
  });
  it("pause stops the interval until resumed", async () => {
    const fetcher = vi.fn(async () => ({}));
    const { getByRole } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    fireEvent.click(getByRole("button", { name: "pause" }));
    const at = fetcher.mock.calls.length;
    await sleep(3200);
    expect(fetcher.mock.calls.length).toBe(at);
  });
  it("unmount aborts the in-flight request", async () => {
    const box: { captured: AbortSignal | null; release: ((v: unknown) => void) | null } = {
      captured: null,
      release: null,
    };
    const fetcher = vi.fn((s: AbortSignal) => {
      box.captured = s;
      return new Promise<unknown>((res) => (box.release = res));
    });
    const { unmount } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(box.captured).toBeTruthy());
    unmount();
    expect(box.captured?.aborted).toBe(true);
    box.release?.({});
  });
  it("hidden tabs skip polls; becoming visible catches up immediately", async () => {
    const fetcher = vi.fn(async () => ({}));
    const { getByTestId } = render(<Probe fetcher={fetcher} />);
    await waitFor(() => expect(fetcher).toHaveBeenCalledTimes(1));
    const desc = Object.getOwnPropertyDescriptor(document, "visibilityState");
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => "hidden",
    });
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
      await sleep(1600);
    });
    const count = fetcher.mock.calls.length;
    expect(getByTestId("state").textContent).toBe("paused");
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => "visible",
    });
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });
    expect(fetcher.mock.calls.length).toBe(count + 1);
    if (desc?.get) Object.defineProperty(document, "visibilityState", desc);
  });
});
