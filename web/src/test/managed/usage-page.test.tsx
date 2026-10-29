/**
 * Usage page (features/managed/usage/Usage.tsx).
 *
 * The filter is the interesting part: the server reads `since`/`until` as
 * epoch milliseconds and the export route reads `format` from the query
 * string of a POST. These tests pin both.
 */
import { describe, expect, it } from "vitest";
import { fireEvent, screen } from "@testing-library/react";
import { Usage } from "../../features/managed/usage/Usage";
import { controlInput } from "../harness";
import { mountManaged, usageFixture, untilCall, type RouteHandler } from "./harness";

const exportFixture = {
  schema: 1,
  path: "/var/lib/harvest/exports/usage-2023-11.jsonl",
  manifest: "/var/lib/harvest/exports/usage-2023-11.jsonl.manifest",
  rows: 3,
  bytes: 48_000,
  sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
  truncated: false,
};

async function mount(routes_: Record<string, RouteHandler | unknown>) {
  return mountManaged(<Usage />, routes_);
}

describe("Usage page", () => {
  it("renders the totals, the per-key table and the per-day table", async () => {
    const { container } = await mount({ "/v1/admin/usage": usageFixture });
    await screen.findByText("Per key");
    // "Requests" is both a metric label and a table header.
    expect(screen.getAllByText("Requests").length).toBeGreaterThanOrEqual(3);
    expect(screen.getByText("Units billed")).toBeInTheDocument();
    expect(screen.getByText("Bytes delivered")).toBeInTheDocument();
    expect(screen.getAllByText("47 KiB").length).toBeGreaterThanOrEqual(3);
    expect(screen.getAllByText("key-1").length).toBeGreaterThanOrEqual(2);
    expect(screen.getByText("t-demo")).toBeInTheDocument();
    expect(screen.getByText("2 / 1 / 0")).toBeInTheDocument();
    expect(screen.getByText("2023-11-14")).toBeInTheDocument();
    expect(screen.getByText(/ledger head seq 7/)).toBeInTheDocument();
    // The UTC-day filter starts empty, so no epoch value is sent.
    expect(controlInput(container, "Since (UTC day)").value).toBe("");
  });

  it("passes key_id and the epoch-millisecond bounds as query params", async () => {
    const { calls, container } = await mount({ "/v1/admin/usage": usageFixture });
    await screen.findByText("Per key");
    const start = calls.length;

    fireEvent.change(screen.getByRole("combobox", { name: "Key" }), {
      target: { value: "key-1" },
    });
    fireEvent.change(controlInput(container, "Since (UTC day)"), {
      target: { value: "2023-11-14" },
    });
    fireEvent.change(controlInput(container, "Until (UTC day)"), {
      target: { value: "2023-11-15" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Apply filter" }));

    const get = await untilCall(
      calls,
      (c) => c.path === "/v1/admin/usage" && c.method === "GET",
      "the filtered usage GET",
      { after: start },
    );
    const qs = new URLSearchParams(get.search.replace(/^\?/, ""));
    expect(qs.get("key_id")).toBe("key-1");
    expect(qs.get("since")).toBe(String(Date.parse("2023-11-14T00:00:00Z")));
    expect(qs.get("until")).toBe(String(Date.parse("2023-11-15T00:00:00Z")));
    expect(qs.get("limit")).toBe("200");
  });

  it("resets the filter to the full window", async () => {
    const { calls, container } = await mount({ "/v1/admin/usage": usageFixture });
    await screen.findByText("Per key");
    fireEvent.change(screen.getByRole("combobox", { name: "Key" }), { target: { value: "key-1" } });
    fireEvent.change(controlInput(container, "Since (UTC day)"), { target: { value: "2023-11-14" } });
    const start = calls.length;
    fireEvent.click(screen.getByRole("button", { name: "Apply filter" }));
    const first = await untilCall(calls, (c) => c.method === "GET", "the first GET", { after: start });
    expect(first.search).toContain("key_id");

    fireEvent.click(screen.getByRole("button", { name: "Reset" }));
    const get = await untilCall(
      calls,
      (c) => c.method === "GET" && !c.search.includes("key_id"),
      "the reset GET",
      { after: start },
    );
    expect(get.search).not.toContain("since");
  });

  it("posts the export with the filter and format as query params", async () => {
    const { calls, container } = await mount({
      "/v1/admin/usage": usageFixture,
      "/v1/admin/usage/export": exportFixture,
    });
    await screen.findByText("Per key");
    fireEvent.change(screen.getByRole("combobox", { name: "Key" }), { target: { value: "key-1" } });
    fireEvent.change(controlInput(container, "Since (UTC day)"), { target: { value: "2023-11-14" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply filter" }));
    await untilCall(calls, (c) => c.method === "GET", "the usage GET");
    const start = calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Export CSV" }));
    const post = await untilCall(
      calls,
      (c) => c.method === "POST" && c.path.startsWith("/v1/admin/usage/export"),
      "the export POST",
      { after: start },
    );
    const qs = new URLSearchParams(post.search.replace(/^\?/, ""));
    expect(qs.get("format")).toBe("csv");
    expect(qs.get("key_id")).toBe("key-1");
    expect(qs.get("since")).toBe(String(Date.parse("2023-11-14T00:00:00Z")));
    // A body here would be ignored — the contract is query-only.
    expect(post.body).toBe("{}");

    const dialog = await screen.findByText(exportFixture.manifest);
    expect(dialog).toBeInTheDocument();
    expect(screen.getByText(exportFixture.path)).toBeInTheDocument();
    expect(screen.getByText(/3 · 47 KiB · sha256/)).toBeInTheDocument();
  });

  it("reports a truncated export", async () => {
    const { findByText } = await mount({
      "/v1/admin/usage": usageFixture,
      "/v1/admin/usage/export": { ...exportFixture, truncated: true, rows: 200 },
    });
    await screen.findByRole("button", { name: "Export JSONL" });
    fireEvent.click(screen.getByRole("button", { name: "Export JSONL" }));
    expect(await findByText(/truncated/)).toBeInTheDocument();
  });

  it("shows an empty state when the window has no rows", async () => {
    const empty = { ...usageFixture, by_key: [], by_day: [], totals: { ...usageFixture.totals, requests: 0 } };
    const { findByText } = await mount({ "/v1/admin/usage": empty });
    expect(await findByText("No usage in this window")).toBeInTheDocument();
  });

  it("shows an error state when the report cannot be read", async () => {
    const { findByText } = await mount({ "/v1/admin/usage": new Error("boom") });
    expect(await findByText("Could not read the usage report")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("shows the skeleton before the report lands", async () => {
    const { container } = await mount({
      "/v1/admin/usage": () => new Promise((r) => setTimeout(() => r(usageFixture), 1000)),
    });
    expect(container.querySelector(".skeleton")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reading…" })).toBeInTheDocument();
    expect(screen.queryByText("Per key")).not.toBeInTheDocument();
  });
});
