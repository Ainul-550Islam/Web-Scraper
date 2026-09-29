import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { CommandPalette, type PaletteItem } from "../../components/Overlays";

const ITEMS: PaletteItem[] = [
  { id: "nav-runs", group: "Navigate", label: "Runs", hint: "history", run: vi.fn() },
  { id: "act-refresh", group: "Actions", label: "Refresh all data", run: vi.fn() },
  { id: "rec-1", group: "Records", label: "target: demo-catalog", hint: "http://x", run: vi.fn() },
];

describe("CommandPalette (Overlays/CommandPalette.tsx)", () => {
  it("groups non-empty sections only; listbox tracks the active option", () => {
    const { getByRole, getAllByRole } = render(
      <CommandPalette items={ITEMS} onClose={() => undefined} />,
    );
    const list = getByRole("listbox");
    expect(getAllByRole("option").length).toBe(3);
    expect(list.getAttribute("aria-activedescendant")).toBe("nav-runs");
  });
  it("search filters by label OR hint and resets the active index", () => {
    const { getAllByRole, getByRole } = render(
      <CommandPalette items={ITEMS} onClose={() => undefined} />,
    );
    fireEvent.change(getByRole("textbox"), { target: { value: "history" } });
    expect(getAllByRole("option").length).toBe(1);
    expect(getByRole("listbox").getAttribute("aria-activedescendant")).toBe("nav-runs");
  });
  it("ArrowDown moves selection; Enter runs the item and closes", () => {
    const onClose = vi.fn();
    const items = ITEMS.map((i) => ({ ...i, run: vi.fn() }));
    const { getByRole } = render(<CommandPalette items={items} onClose={onClose} />);
    fireEvent.keyDown(getByRole("textbox"), { key: "ArrowDown" });
    expect(getByRole("listbox").getAttribute("aria-activedescendant")).toBe("act-refresh");
    fireEvent.keyDown(getByRole("textbox"), { key: "Enter" });
    expect(items[1]?.run).toHaveBeenCalledTimes(1);
    expect(items[0]?.run).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
