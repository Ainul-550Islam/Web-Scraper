import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Pagination } from "../../components/ui";

describe("Pagination (components/ui/Pagination.tsx)", () => {
  it("prints the visible range and clamps to real pages", () => {
    const { getByText } = render(
      <Pagination total={60} limit={25} offset={0} onPage={() => undefined} />,
    );
    expect(getByText("1–25 of 60")).toBeTruthy();
    expect(getByText("1 / 3")).toBeTruthy();
  });
  it("prev disabled on first page and next jumps by a real offset", () => {
    const onPage = vi.fn();
    const { getByLabelText } = render(
      <Pagination total={60} limit={25} offset={0} onPage={onPage} />,
    );
    expect(getByLabelText("previous page")).toBeDisabled();
    fireEvent.click(getByLabelText("next page"));
    expect(onPage).toHaveBeenCalledWith(25);
  });
  it("next is disabled on the last page", () => {
    const { getByLabelText } = render(
      <Pagination total={60} limit={25} offset={50} onPage={() => undefined} />,
    );
    expect(getByLabelText("next page")).toBeDisabled();
  });
  it("says so for zero entries", () => {
    const { getByText } = render(<Pagination total={0} limit={25} offset={0} onPage={() => undefined} />);
    expect(getByText("no entries")).toBeTruthy();
  });
});
