import { describe, expect, it } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import App from "../../app/App";
import { mountPage } from "../harness";
import { invalidateCache } from "../../lib/api";

describe("responsive navigation (§37)", () => {
  it("the mobile-only menu button opens the rail", async () => {
    invalidateCache();
    mountPage(<App />);
    const menu = await screen.findByRole("button", { name: "Open navigation" });
    expect(menu.className).toContain("mobile-only"); // present in DOM for narrow viewports
    fireEvent.click(menu);
    await waitFor(() => expect(document.querySelector("nav.rail")).toHaveAttribute("data-open", "true"));
  });

  it("collapse state is exposed on the shell grid", async () => {
    invalidateCache();
    mountPage(<App />);
    const toggle = await screen.findByRole("button", { name: "Collapse navigation" });
    fireEvent.click(toggle);
    await waitFor(() => expect(document.querySelector(".shell")).toHaveAttribute("data-collapsed", "true"));
  });
});
