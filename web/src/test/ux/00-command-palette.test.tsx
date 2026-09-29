import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import App from "../../app/App";
import { mountPage, until } from "../harness";
import { invalidateCache } from "../../lib/api";

describe("command palette (§26)", () => {
  it("opens on Ctrl+K, filters, and Enter navigates to the real route", async () => {
    window.location.hash = "#/";
    invalidateCache();
    mountPage(<App />);
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Command palette" });
    const input = within(dialog).getByRole("textbox");
    fireEvent.change(input, { target: { value: "proxies" } });
    const option = await within(dialog).findByRole("option", { name: /proxy pool/i });
    expect(option).toHaveAttribute("aria-selected", "true");
    fireEvent.keyDown(input, { key: "Enter" });
    await until(() => window.location.hash.includes("proxies"));
    expect(window.location.hash).toBe("#/proxies");
  });

  it("states honestly that search is not full-text", async () => {
    invalidateCache();
    mountPage(<App />);
    fireEvent.keyDown(window, { key: "k", ctrlKey: true });
    const dialog = await screen.findByRole("dialog", { name: "Command palette" });
    fireEvent.change(within(dialog).getByRole("textbox"), { target: { value: "zzzzz" } });
    expect(await within(dialog).findByText(/not full-text/i)).toBeInTheDocument();
  });
});
