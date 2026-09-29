import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, waitFor } from "@testing-library/react";
import { SearchInput } from "../../components/patterns";

describe("SearchInput (patterns/SearchInput.tsx)", () => {
  it("debounces by the configured interval and keeps a labelled searchbox", async () => {
    const onChange = vi.fn();
    const { getByRole } = render(
      <SearchInput value="" onChange={onChange} label="filter" debounceMs={10} />,
    );
    const el = getByRole("searchbox");
    expect(el.getAttribute("aria-label")).toBe("filter");
    fireEvent.change(el, { target: { value: "queue" } });
    expect(onChange).not.toHaveBeenCalled();
    await waitFor(() => expect(onChange).toHaveBeenCalledWith("queue"));
  });
});
