import { describe, expect, it } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import Overview from "../../features/overview/Overview";
import { mountPage, stats } from "../harness";
import { fmtNum } from "../../lib/format";

describe("Overview page", () => {
  it("renders engine metrics straight from GET /stats", async () => {
    const { container, findByText } = await mountPage(<Overview />);
    await findByText("Records written");
    const value = (label: string) => {
      const card = [...container.querySelectorAll(".card.metric")].find((c) =>
        c.querySelector(".metric-label")?.textContent?.includes(label),
      );
      return card?.querySelector(".metric-value")?.textContent?.trim();
    };
    // the exact server numbers, formatted through the app's own fmtNum
    expect(value("Records written")).toBe(fmtNum(stats.metrics.records_written));
    expect(value("Pages fetched")).toBe(fmtNum(stats.metrics.pages_fetched));
  });

  it("shows the live engine state", async () => {
    const { container } = await mountPage(<Overview />);
    await waitFor(() => {
      const card = [...container.querySelectorAll(".card.metric")].find((c) =>
        c.querySelector(".metric-label")?.textContent?.includes("Engine"),
      );
      expect(card?.textContent).toMatch(/running|idle|offline/);
    });
  });

  it("labels the metric grid with real card names", async () => {
    const { findByText } = await mountPage(<Overview />);
    expect(await findByText("Records written")).toBeInTheDocument();
    expect(screen.getByText("Queue depth")).toBeInTheDocument();
  });
});
