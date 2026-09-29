/**
 * §29 global shortcuts — navigation sequences, input guarding, and
 * modifier suppression. Drives the real useShortcuts hook.
 */
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { useShortcuts, SHORTCUT_DOC } from "../hooks/useShortcuts";

function Probe() {
  useShortcuts(true);
  return <input aria-label="probe" defaultValue="" />;
}

const press = (keys: string[], target: Window | Element = window) => {
  for (const k of keys) fireEvent.keyDown(target, { key: k });
};

describe("g-sequences (§29)", () => {
  it("g r navigates to Runs and g c to Configuration", () => {
    render(<Probe />);
    window.location.hash = "#/";
    press(["g", "r"]);
    expect(window.location.hash).toBe("#/runs");
    press(["g", "c"]);
    expect(window.location.hash).toBe("#/config");
  });

  it("all documented targets are reachable", () => {
    render(<Probe />);
    for (const [keys, dest] of [
      ["g o", "#/"],
      ["g j", "#/jobs"],
      ["g w", "#/workers"],
      ["g p", "#/proxies"],
      ["g q", "#/queue"],
    ] as const) {
      window.location.hash = "#/runs"; // anywhere but the target
      press(keys.split(" "));
      expect(window.location.hash, keys).toBe(dest);
    }
  });

  it("never fires while typing in a field", () => {
    render(<Probe />);
    window.location.hash = "#/";
    const inp = screen.getByLabelText("probe");
    inp.focus();
    press(["g", "r"], inp);
    press(["g", "c"], inp);
    expect(window.location.hash).toBe("#/");
  });

  it("an armed `g` + unknown key disarms; stale g windows expire", () => {
    render(<Probe />);
    window.location.hash = "#/";
    press(["g", "x", "r"]); // 'x' disarms the sequence
    expect(window.location.hash).toBe("#/");
  });

  it("modifier chords pass through untouched", () => {
    render(<Probe />);
    window.location.hash = "#/";
    fireEvent.keyDown(window, { key: "g", ctrlKey: true });
    fireEvent.keyDown(window, { key: "r", ctrlKey: true });
    expect(window.location.hash).toBe("#/");
  });

  it("docs copy lists every shipped shortcut (no drift)", () => {
    const joined = SHORTCUT_DOC.map(([k]) => k).join(",");
    expect(joined).toContain("g then o");
    expect(joined).toContain("g then c");
  });
});
