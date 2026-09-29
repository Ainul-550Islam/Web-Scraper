import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import Activity from "../../features/activity/Activity";
import { mountPage, until } from "../harness";

describe("Activity page (delta polling)", () => {
  it("fetches a bounded ring page immediately", async () => {
    const { container, calls } = await mountPage(<Activity />);
    await waitFor(() =>
      expect(container.querySelectorAll('[aria-label="engine event feed"] > li').length).toBeGreaterThan(0),
    );
    expect(calls.some((c) => c.path === "/events" && c.search.includes("limit=64"))).toBe(true);
  });

  it("advances the since_seq cursor to the server's last_seq", async () => {
    const { calls, container } = await mountPage(<Activity />);
    await waitFor(() =>
      expect(container.querySelectorAll('[aria-label="engine event feed"] > li').length).toBeGreaterThan(0),
    );
    // first delta consumed seq ≤ 14 → next 4s poll asks for since_seq=14
    await until(() => calls.some((c) => c.path === "/events" && c.search.includes("since_seq=14")), { timeout: 6000 });
  });

  it("offers pause without dropping the view", async () => {
    const { getByRole, container } = await mountPage(<Activity />);
    await waitFor(() =>
      expect(container.querySelectorAll('[aria-label="engine event feed"] > li').length).toBeGreaterThan(0),
    );
    const pause = getByRole("button", { name: /pause/i });
    fireEvent.click(pause);
    expect(pause.getAttribute("aria-pressed")).toBe("true"); // explicit pressed state on the control
  });
});
