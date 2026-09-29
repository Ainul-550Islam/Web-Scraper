// ApiExplorer page: renders the manifest verbatim, GET/POST segmentation is
// real filtering, and the honesty note stays on the page.

import { describe, expect, it } from "vitest";
import { fireEvent, screen, within } from "@testing-library/react";
import { mountPage } from "../harness";
import ApiExplorer from "../../features/api/ApiExplorer";
import { API_ROUTES } from "../../lib/apiRoutes";

describe("ApiExplorer", () => {
  it("lists every manifest route with method + path", async () => {
    await mountPage(<ApiExplorer />);
    expect(await screen.findByText("API Explorer")).toBeTruthy();
    const rows = () => within(document.querySelector("table")!);
    for (const p of ["/health", "/runs", "/alerts/ack", "/ops/note"]) {
      expect(rows().getByText(p, { selector: "code" })).toBeTruthy();
    }
    expect(screen.getByText(`${API_ROUTES.length} routes`, { exact: false })).toBeTruthy();
    for (const needle of [
      "GET", "POST", "bearer if set", "Copy route manifest", "How this page stays honest",
    ]) {
      expect(document.body.textContent).toContain(needle);
    }
  });

  it("POST filter leaves only the two real mutations", async () => {
    await mountPage(<ApiExplorer />);
    await screen.findByText("API Explorer");
    const rows = () => within(document.querySelector("table")!);
    fireEvent.click(screen.getByRole("button", { name: /POST/ }));
    expect(rows().getByText("/alerts/ack", { selector: "code" })).toBeTruthy();
    expect(rows().getByText("/ops/note", { selector: "code" })).toBeTruthy();
    expect(rows().queryAllByText("/health", { selector: "code" })).toHaveLength(0);
  });
});
