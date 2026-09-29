import { describe, expect, it } from "vitest";
import { act, fireEvent, render } from "@testing-library/react";
import { KEY, ThemeProvider, useTheme } from "../../hooks/useTheme";

function Probe() {
  const { theme, resolved, setTheme } = useTheme();
  return (
    <>
      <output data-testid="t">{theme}</output>
      <output data-testid="r">{resolved}</output>
      <button onClick={() => setTheme("system")}>use system</button>
      <button onClick={() => setTheme("light")}>use light</button>
    </>
  );
}

function stubMedia(initial: { light: boolean }) {
  const state = { ...initial };
  const listeners: Array<() => void> = [];
  const prev = window.matchMedia;
  window.matchMedia = ((q: string) => ({
    get matches() {
      return q.includes("light") ? state.light : !state.light;
    },
    media: q,
    addEventListener: (_: string, fn: () => void) => listeners.push(fn),
    removeEventListener: (_: string, fn: () => void) => {
      const i = listeners.indexOf(fn);
      if (i >= 0) listeners.splice(i, 1);
    },
  })) as unknown as typeof window.matchMedia;
  return {
    restore: () => {
      window.matchMedia = prev;
    },
    flip: () => {
      state.light = !state.light;
      listeners.forEach((f) => f());
    },
  };
}

describe("useTheme + ThemeProvider (hooks/useTheme)", () => {
  it("defaults to dark and writes ONLY the theme key on change", () => {
    localStorage.clear();
    const { getByRole, getByTestId } = render(
      <ThemeProvider><Probe /></ThemeProvider>,
    );
    expect(getByTestId("t").textContent).toBe("dark");
    fireEvent.click(getByRole("button", { name: "use light" }));
    expect(getByTestId("r").textContent).toBe("light");
    expect(document.documentElement.dataset["theme"]).toBe("light");
    expect(Object.keys(localStorage)).toEqual([KEY]);
    expect(localStorage.getItem(KEY)).toBe("light");
  });
  it("system resolves through the media query and live-updates on change", () => {
    const media = stubMedia({ light: false });
    localStorage.clear();
    try {
      const { getByRole, getByTestId } = render(
        <ThemeProvider><Probe /></ThemeProvider>,
      );
      fireEvent.click(getByRole("button", { name: "use system" }));
      expect(getByTestId("r").textContent).toBe("dark");
      act(() => media.flip());
      expect(document.documentElement.dataset["theme"]).toBe("light");
    } finally {
      media.restore();
    }
  });
  it("garbage in storage falls back to dark without crashing", () => {
    localStorage.clear();
    localStorage.setItem(KEY, "neon");
    const { getByTestId } = render(
      <ThemeProvider><Probe /></ThemeProvider>,
    );
    expect(getByTestId("t").textContent).toBe("dark");
    localStorage.clear();
  });
});
