import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import Alerts from "../../features/alerts/Alerts";
import { mountPage, until } from "../harness";

describe("Alerts page", () => {
  it("acks a single alert via POST /alerts/ack?event=<key> behind confirmation", async () => {
    const seeded = {
      cooldown_secs: 60,
      fired: 3,
      notified: 2,
      min_severity: "",
      items: [{ key: "block-rate", severity: "critical", message: "block rate 40%", ts: 1789963490 }],
    };
    const { calls, findByText } = await mountPage(<Alerts />, {
      "/alerts": seeded,
      "/alerts/ack": { acked: 1, event: "block-rate", result: "ok" },
    });
    expect(await findByText("block rate 40%")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Acknowledge" }));
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /Acknowledge/ }));

    await until(() => calls.some((c) => c.method === "POST" && c.path === "/alerts/ack" && c.search.includes("event=block-rate")));
    expect(await screen.findByText(/1 entr.* acknowledged/i)).toBeInTheDocument();
  });

  it("keeps the config-derived threshold copy honest", async () => {
    const { findByText, findAllByText } = await mountPage(<Alerts />);
    expect(await findByText(/No live alerts/i)).toBeInTheDocument();
    expect((await findAllByText(/cooldown/i)).length).toBeGreaterThan(0);
  });
});
