import { describe, expect, it } from "vitest";
import { fireEvent, waitFor } from "@testing-library/react";
import Runs from "../../features/runs/Runs";
import { mountPage, until } from "../harness";
import { seededRuns } from "./helpers";

describe("pagination (§28)", () => {
  it("advances the server offset param on next-page", async () => {
    const { container, calls, getByLabelText } = await mountPage(<Runs />, { "/runs": seededRuns() });
    await waitFor(() => expect(container.querySelectorAll("tbody tr").length).toBeGreaterThan(15));
    fireEvent.click(getByLabelText("next page"));
    await until(() => calls.some((c) => c.search.includes("offset=15")));
  });

  it("pager reports the real total range", async () => {
    const { findByText } = await mountPage(<Runs />, { "/runs": seededRuns() });
    expect(await findByText("1–15 of 22")).toBeInTheDocument();
  });
});
