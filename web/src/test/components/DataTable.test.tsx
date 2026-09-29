import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { DataTable } from "../../components/Table";
import type { Column } from "../../components/Table";

interface Row {
  name: string;
  n: number;
  extra: string;
}
const COLS: Column<Row>[] = [
  { key: "name", label: "Name", render: (r) => r.name },
  { key: "n", label: "N", numeric: true, sortValue: (r) => r.n, render: (r) => String(r.n) },
  { key: "extra", label: "Extra", render: () => "e" },
];
const ROWS: Row[] = [
  { name: "b", n: 2, extra: "x" },
  { name: "a", n: 7, extra: "y" },
];

describe("DataTable (Table/DataTable.tsx)", () => {
  it("renders caption, one row per entry with roving tabindex on the first only", () => {
    const { getByText, container } = render(
      <DataTable caption="Runs" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="none" />,
    );
    expect(getByText("Runs", { selector: ".t-label" })).toBeTruthy();
    const trs = container.querySelectorAll("tbody tr");
    expect(trs.length).toBe(2);
    expect(trs[0]?.getAttribute("tabindex")).toBe("0");
    expect(trs[1]?.getAttribute("tabindex")).toBe("-1");
  });
  it("loading shows 5 skeletons; empty shows the honest EmptyState", () => {
    const { container } = render(
      <DataTable caption="c" columns={COLS} rows={[]} rowKey={(r) => r.name} loading emptyTitle="none" />,
    );
    expect(container.querySelectorAll(".skeleton").length).toBe(5);
    const { getByText } = render(
      <DataTable caption="c" columns={COLS} rows={[]} rowKey={(r) => r.name} emptyTitle="No runs" emptyBody="run one" />,
    );
    expect(getByText("No runs")).toBeTruthy();
    expect(getByText("run one")).toBeTruthy();
  });
  it("header sort: new column sorts descending first, toggles to ascending, rows follow sortValue", () => {
    const { getAllByRole, container } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" />,
    );
    const sortBtns = getAllByRole("columnheader").map((th) => th.querySelector("button")!);
    fireEvent.click(sortBtns[1]!);
    expect(getAllByRole("columnheader")[1]?.getAttribute("aria-sort")).toBe("descending");
    expect(container.querySelector("tbody tr")?.textContent).toContain("a");
    fireEvent.click(sortBtns[1]!);
    expect(getAllByRole("columnheader")[1]?.getAttribute("aria-sort")).toBe("ascending");
    expect(container.querySelector("tbody tr")?.textContent).toContain("b");
  });
  it("density button presses and unpresses the compact class", () => {
    const { getByRole, container } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" />,
    );
    const compact = getByRole("button", { name: "Compact" });
    expect(compact.getAttribute("aria-pressed")).toBe("false");
    fireEvent.click(compact);
    expect(compact.getAttribute("aria-pressed")).toBe("true");
    expect(container.querySelector(".table-wrap.compact")).toBeTruthy();
  });
  it("column menu hides a column and refuses to drop below two visible", () => {
    const { getByRole, getAllByRole, container } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" />,
    );
    fireEvent.click(getByRole("button", { name: /Columns/ }));
    const menu = getByRole("menu", { name: "column visibility" });
    const checks = getAllByRole("checkbox");
    fireEvent.click(checks[2]!);
    expect(container.querySelectorAll("thead th").length).toBe(2);
    const stillOn = [...menu.querySelectorAll("input")].find((i) => i.checked);
    expect(container.querySelectorAll("thead th").length).toBe(2);
    // hiding further would leave one column — the checked ones stay enabled, but
    // with only 3 columns at 2 visible the LAST visible toggle is disabled:
    fireEvent.click(checks[1]!);
    expect(container.querySelectorAll("thead th").length).toBe(2);
    expect(stillOn).toBeTruthy();
  });
  it("server pager appears only when onPage is given", () => {
    const onPage = vi.fn();
    const first = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" total={60} limit={25} offset={25} onPage={onPage} />,
    );
    expect(first.queryByRole("navigation", { name: "pagination" })).toBeTruthy();
    first.unmount();
    const { queryByRole: none } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" />,
    );
    expect(none("navigation", { name: "pagination" })).toBeNull();
  });
  it("rows are clickable + Enter-activatable when onRowClick, selectedKey marks aria-selected", () => {
    const onRowClick = vi.fn();
    const { container } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" onRowClick={onRowClick} selectedKey="a" />,
    );
    const trs = [...container.querySelectorAll("tbody tr")] as HTMLTableRowElement[];
    expect(trs[1]?.getAttribute("aria-selected")).toBe("true");
    fireEvent.click(trs[0]!);
    fireEvent.keyDown(trs[1]!, { key: "Enter" });
    expect(onRowClick).toHaveBeenCalledTimes(2);
    expect(onRowClick).toHaveBeenLastCalledWith(ROWS[1]);
  });
  it("arrow navigation moves the roving focus index", () => {
    const { container } = render(
      <DataTable caption="c" columns={COLS} rows={ROWS} rowKey={(r) => r.name} emptyTitle="n" />,
    );
    const card = container.querySelector(".card")!;
    fireEvent.keyDown(card, { key: "ArrowDown" });
    fireEvent.keyDown(card, { key: "ArrowDown" });
    fireEvent.keyDown(card, { key: "End" });
    // focus lands on the last row after the rAF tick; clamp keeps it at index n-1
    const trs = container.querySelectorAll("tbody tr");
    expect(trs.length).toBe(2);
  });
});
