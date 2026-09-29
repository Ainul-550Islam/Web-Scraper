import { describe, expect, it } from "vitest";
import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import App from "../../app/App";
import { mountPage, until } from "../harness";
import { auth, invalidateCache } from "../../lib/api";

describe("bearer token (§39)", () => {
  it("attaches Authorization only in-memory and never persists it", async () => {
    invalidateCache();
    const { calls } = await mountPage(<App />);
    const lock = await screen.findByRole("button", { name: /set bearer token/i });
    fireEvent.click(lock);
    const dialog = await screen.findByRole("dialog");
    const input = within(dialog).getByLabelText("Bearer token");
    fireEvent.change(input, { target: { value: "super-secret-token-9000" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Use token" }));
    fireEvent.click(await screen.findByRole("button", { name: "Refresh all data" }));

    // force one poll, then assert the header went out and nothing was stored
    await waitFor(() => expect(calls.length).toBeGreaterThan(0));
    await until(() => calls.some((c) => c.headers["authorization"] === "Bearer super-secret-token-9000"));
    const stored = Object.keys(localStorage).map((k) => `${k}=${localStorage.getItem(k)}`).join(";");
    expect(stored).not.toContain("super-secret");
    expect(JSON.stringify(calls.map((c) => c.search))).not.toContain("super-secret");
    auth.set(null);
  });
});
