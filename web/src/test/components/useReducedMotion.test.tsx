import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { useReducedMotion } from "../../components/AppShell/useReducedMotion";

function stubMedia(matches: boolean) {
  const prev = window.matchMedia;
  window.matchMedia = ((q: string) => ({
    matches,
    media: q,
    addEventListener: () => undefined,
    removeEventListener: () => undefined,
  })) as unknown as typeof window.matchMedia;
  return () => {
    window.matchMedia = prev;
  };
}

function Probe() {
  return <output>{useReducedMotion() ? "reduced" : "full"}</output>;
}

describe("useReducedMotion (AppShell/useReducedMotion.tsx)", () => {
  it("tracks the prefers-reduced-motion media query", () => {
    const on = stubMedia(true);
    expect(render(<Probe />).getByText("reduced")).toBeTruthy();
    on();
    const off = stubMedia(false);
    expect(render(<Probe />).getByText("full")).toBeTruthy();
    off();
  });
});
