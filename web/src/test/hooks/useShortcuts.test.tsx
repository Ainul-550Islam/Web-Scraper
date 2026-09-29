import { describe, expect, it, vi } from "vitest";
import { render } from "@testing-library/react";
import { GOTO, isEditable, useShortcuts } from "../../hooks/useShortcuts";

function Probe({ enabled = true }: { enabled?: boolean }) {
  useShortcuts(enabled);
  return <i />;
}

const key = (k: string, init: Partial<KeyboardEventInit> = {}) =>
  window.dispatchEvent(new KeyboardEvent("keydown", { key: k, ...init }));

describe("useShortcuts (hooks/useShortcuts)", () => {
  it("the GOTO map covers the documented keys and nothing else", () => {
    expect(Object.keys(GOTO).sort()).toEqual(["c", "j", "o", "p", "q", "r", "w"]);
    expect(GOTO.o).toBe("/");
    expect(GOTO.c).toBe("/config");
  });
  it("g-then-r jumps; an unknown key disarms the window", () => {
    window.location.hash = "";
    render(<Probe />);
    key("g");
    key("r");
    expect(window.location.hash).toBe("#/runs");
    window.location.hash = "";
    key("g");
    key("x"); // unknown second key disarms (the header's documented rule)
    key("r");
    expect(window.location.hash).toBe("");
  });
  it("the window is 1.5 s: a late second key does nothing", () => {
    vi.useFakeTimers();
    try {
      window.location.hash = "";
      render(<Probe />);
      key("g");
      vi.advanceTimersByTime(1600);
      key("j");
      expect(window.location.hash).toBe("");
    } finally {
      vi.useRealTimers();
    }
  });
  it("modified keys, editable targets, and enabled=false are all inert", () => {
    window.location.hash = "";
    const { unmount } = render(<Probe />);
    key("g");
    key("r", { ctrlKey: true });
    expect(window.location.hash).toBe("");
    unmount();
    render(<Probe enabled={false} />);
    key("g");
    key("q");
    expect(window.location.hash).toBe("");
  });
});

describe("isEditable (hooks/useShortcuts/isEditable)", () => {
  it("text-bearing elements are editable, containers and window are not", () => {
    expect(isEditable(document.createElement("input"))).toBe(true);
    expect(isEditable(document.createElement("textarea"))).toBe(true);
    expect(isEditable(document.createElement("select"))).toBe(true);
    const div = document.createElement("div");
    expect(isEditable(div)).toBe(false); // strict boolean, never undefined
    Object.defineProperty(div, "isContentEditable", { value: true });
    expect(isEditable(div)).toBe(true);
    expect(isEditable(window)).toBe(false);
    expect(isEditable(null)).toBe(false);
  });
});
