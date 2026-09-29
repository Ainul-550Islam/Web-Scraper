import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import Runs from "../../features/runs/Runs";
import { mountPage, until } from "../harness";

describe("Runs page", () => {
  it("renders the server-side page window from GET /runs", async () => {
    const { getByText, findByText } = await mountPage(<Runs />);
    expect(await findByText("demo-run")).toBeInTheDocument();
    expect(getByText("Cancelled")).toBeInTheDocument(); // full real status filter set
  });

  it("passes the status filter through as a real query param", async () => {
    const { calls, findByText } = await mountPage(<Runs />);
    await findByText("demo-run");
    fireEvent.click(screen.getByRole("button", { name: "Errors" }));
    await until(() => calls.some((c) => c.path === "/runs" && c.search.includes("status=error")));
  });

  it("opens the run drawer on row click", async () => {
    const { container, findByText } = await mountPage(<Runs />);
    await findByText("demo-run");
    const row = container.querySelector("tbody tr");
    expect(row).toBeTruthy();
    fireEvent.click(row!);
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
  });

  it("keeps sort and paging server-driven", async () => {
    const { calls, findByText } = await mountPage(<Runs />);
    await findByText("demo-run");
    expect(calls.some((c) => c.search.includes("limit=") && c.search.includes("offset="))).toBe(true);
    const sortGroup = screen.getByRole("group", { name: "sort" });
    fireEvent.click(within(sortGroup).getByRole("button", { name: "Written" }));
    await until(() => calls.some((c) => c.search.includes("sort=written")));
  });
});
