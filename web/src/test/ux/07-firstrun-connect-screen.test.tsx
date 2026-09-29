import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import App from "../../app/App";
import { DemoBanner } from "../../components/AppShell";
import { ConnectScreen } from "../../app/ConnectScreen";
import { mountPage } from "../harness";
import { invalidateCache, setApiBase } from "../../lib/api";

describe("first-run connect screen (§3)", () => {
  it("blocks with a connect panel when the API is unreachable, and steps aside on demand", async () => {
    invalidateCache();
    mountPage(<App />, { __all: new TypeError("Failed to fetch") });
    expect(await screen.findByText("Harvest Console", {}, { timeout: 3000 })).toBeInTheDocument();
    expect(screen.getByLabelText("API endpoint")).toHaveValue("/api");
    expect(screen.getByRole("button", { name: /Connect & probe health/i })).toBeInTheDocument();
    // no fabricated version anywhere
    expect(document.body.textContent).not.toMatch(/v?\d+\.\d+\.\d+ \(harvest\)/);
    fireEvent.click(screen.getByRole("button", { name: /Open console anyway/i }));
    await waitFor(() => expect(document.querySelector("nav.rail")).toBeInTheDocument());
  });

  it("probes the endpoint and reports the real health verdict", async () => {
    const onConnected = vi.fn();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        new Response(JSON.stringify({ status: "ok", running: true }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      ) as unknown as typeof fetch,
    );
    render(<ConnectScreen onConnected={onConnected} />);
    fireEvent.click(screen.getByRole("button", { name: /Connect & probe health/i }));
    expect(await screen.findByText(/health ok/i, {}, { timeout: 3000 })).toBeInTheDocument();
    expect(screen.getByText(/version is not exposed/i)).toBeInTheDocument();
    await waitFor(() => expect(onConnected).toHaveBeenCalled());
    localStorage.removeItem("harvest.console.api");
    setApiBase(null, false);
    vi.unstubAllGlobals();
  });

  it("never renders the DEMO badge outside mock mode, and states it loudly inside", () => {
    const { unmount, container } = render(<DemoBanner demo={false} />);
    expect(container).toBeEmptyDOMElement();
    unmount();
    render(<DemoBanner demo />);
    expect(screen.getByRole("status")).toHaveTextContent("DEMO MODE");
  });
});
