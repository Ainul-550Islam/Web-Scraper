import { describe, expect, it, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { useDebounced } from "../../hooks/useDebounced";

function Probe({ value }: { value: string }) {
  return <output>{useDebounced(value, 50)}</output>;
}

describe("useDebounced (hooks/useDebounced)", () => {
  it("delays adoption by the given ms and coalesces bursts", () => {
    vi.useFakeTimers();
    try {
      const { rerender, getByText } = render(<Probe value="a" />);
      rerender(<Probe value="b" />);
      expect(getByText("a")).toBeTruthy(); // not yet
      rerender(<Probe value="c" />);
      act(() => void vi.advanceTimersByTime(49));
      expect(getByText("a")).toBeTruthy(); // one tick early: still old
      act(() => void vi.advanceTimersByTime(1));
      expect(getByText("c")).toBeTruthy(); // only the newest value lands
    } finally {
      vi.useRealTimers();
    }
  });
});
