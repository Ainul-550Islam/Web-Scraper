import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import Runs from "../../features/runs/Runs";
import Overview from "../../features/overview/Overview";
import { mountPage } from "../harness";

describe("connection states (§30, §35)", () => {
  it("network failure → offline state with a retry affordance, no stack traces", async () => {
    const { container } = await mountPage(<Overview />, {
      __all: new TypeError("Failed to fetch"),
      "/health": new TypeError("Failed to fetch"),
      "/stats": new TypeError("Failed to fetch"),
      "/runtime": new TypeError("Failed to fetch"),
      "/diagnostics": new TypeError("Failed to fetch"),
      "/events": new TypeError("Failed to fetch"),
      "/config": new TypeError("Failed to fetch"),
      "/alerts": new TypeError("Failed to fetch"),
    });
    await waitFor(() => expect(screen.getByText("Retry")).toBeInTheDocument(), { timeout: 4000 });
    expect(container.textContent).not.toMatch(/TypeError\s+at|\n\s+at \w/);
  });

  it("never-resolving first fetch keeps the table in a skeleton state", async () => {
    const { container } = await mountPage(<Runs />, {
      "/runs": new Promise<never>(() => undefined),
      "/recent": new Promise<never>(() => undefined),
    });
    await waitFor(() => expect(container.querySelectorAll(".skeleton").length).toBeGreaterThan(0));
  });

  it("server 401 surfaces the API message, never a raw body dump", async () => {
    const { findByText, container } = await mountPage(<Runs />, {
      "/runs": {
        __status: 401,
        body: { error: { code: "unauthorized", message: "bearer token required for non-loopback bind" }, request_id: "req-77" },
      },
    });
    expect(await findByText(/bearer token required/i)).toBeInTheDocument();
    expect(container.textContent).not.toMatch(/"error":\s*\{/); // parsed, not dumped
  });
});
