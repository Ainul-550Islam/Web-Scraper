import { describe, expect, it, vi } from "vitest";
import { buildPaletteItems } from "../../components/AppShell/buildPaletteItems";
import { ROUTES } from "../../app/routes";

function fakeConn() {
  return {
    refreshAll: vi.fn(),
    config: { targets: [{ id: "demo-catalog", start_url: "http://demo.test/catalog" }] },
    alerts: { items: [{ kind: "high_block_rate" }] },
  } as unknown as Parameters<typeof buildPaletteItems>[0];
}

describe("buildPaletteItems (AppShell/buildPaletteItems.tsx)", () => {
  it("emits one Navigate item per real route, two actions, one Record per loaded row", () => {
    const items = buildPaletteItems(fakeConn(), vi.fn());
    const byGroup = (g: string) => items.filter((i) => i.group === g);
    expect(byGroup("Navigate").length).toBe(ROUTES.length);
    expect(byGroup("Actions").map((i) => i.id)).toEqual(["act-refresh", "act-theme"]);
    expect(byGroup("Records").map((i) => i.id)).toEqual(["rec-t-demo-catalog", "rec-alert-0"]);
    expect(byGroup("Navigate")[0]?.label).toBe("Overview");
  });
  it("record search deep-links with the URL-encoded id; actions wire through", () => {
    const conn = fakeConn();
    const toggleTheme = vi.fn();
    const items = buildPaletteItems(conn, toggleTheme);
    items.find((i) => i.id === "act-theme")?.run();
    expect(toggleTheme).toHaveBeenCalledTimes(1);
    items.find((i) => i.id === "act-refresh")?.run();
    expect(conn.refreshAll).toHaveBeenCalledTimes(1);
    window.location.hash = "";
    items.find((i) => i.id === "rec-t-demo-catalog")?.run();
    expect(decodeURIComponent(window.location.hash)).toContain("/config?target=demo-catalog");
  });
});
