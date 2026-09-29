import { describe, expect, it } from "vitest";
import { screen } from "@testing-library/react";
import Workers from "../../features/workers/Workers";
import QueuePage from "../../features/queue/QueuePage";
import { mountPage } from "../harness";

describe("Workers + Queue pages", () => {
  it("shows worker snapshot state", async () => {
    await mountPage(<Workers />);
    expect(await screen.findByText(/healthy|degraded|unavailable/i)).toBeInTheDocument();
  });

  it("surfaces the queue state even when the engine reports unavailable", async () => {
    const { findByText } = await mountPage(<QueuePage />, { "/queue": { state: "unavailable" } });
    expect(await findByText(/unavailable/i)).toBeInTheDocument();
  });

  it("renders the queue metric grid from the real stats map", async () => {
    const { findByText } = await mountPage(<QueuePage />);
    expect(await findByText("Depth (waiting)")).toBeInTheDocument();
    expect(screen.getByText("Inflight")).toBeInTheDocument();
  });

  it("pops per-target pressure rows from per_target_active pairs", async () => {
    const { findByText } = await mountPage(<QueuePage />);
    expect(await findByText("Per-target pressure")).toBeInTheDocument();
  });
});
