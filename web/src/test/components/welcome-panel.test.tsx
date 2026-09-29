// WelcomePanel: shows only after the engine has answered, with real config
// values; its "demo" affordance is a copyable command (there is no run POST).

import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, screen } from "@testing-library/react";
import { mountPage } from "../harness";
import { WelcomePanel } from "../../components/AppShell";
import config from "../fixtures/config.json";

describe("WelcomePanel", () => {
  afterEach(cleanup);

  it("renders live values, not placeholders", async () => {
    const onStart = vi.fn();
    const onDismiss = vi.fn();
    await mountPage(<WelcomePanel onStartTour={onStart} onDismiss={onDismiss} />);
    expect(await screen.findByText(/Welcome to Harvest/)).toBeTruthy();
    expect(screen.getByText("cargo run --release -- run --config examples/config.basic.json")).toBeTruthy();
    expect(screen.getByRole("button", { name: /Open Dashboard/ })).toBeTruthy();
    expect(screen.getByRole("button", { name: /Take the tour/ })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Take the tour/ }));
    expect(onStart).toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: /Don.t show again/ }));
    expect(onDismiss).toHaveBeenCalled();
    // capabilities line reflects fixture numbers, no invented counts
    expect(document.body.textContent).toContain(`${(config as { targets: unknown[] }).targets.length} target(s)`);
  });

  it("offers no button that would fake a run", async () => {
    await mountPage(<WelcomePanel onStartTour={() => {}} onDismiss={() => {}} />);
    await screen.findByText(/Welcome to Harvest/);
    expect(screen.queryByRole("button", { name: /Run Demo/i })).toBeNull();
  });
});
