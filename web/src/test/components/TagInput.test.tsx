import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { TagInput } from "../../components/ui";

describe("TagInput (components/ui/TagInput.tsx)", () => {
  it("commit on Enter appends the trimmed value once and clears the draft", () => {
    const onChange = vi.fn();
    const { getByLabelText } = render(
      <TagInput values={["css"]} onChange={onChange} placeholder="op" />,
    );
    const draft = getByLabelText("add value");
    fireEvent.change(draft, { target: { value: " regex " } });
    fireEvent.keyDown(draft, { key: "Enter" });
    expect(onChange).toHaveBeenCalledWith(["css", "regex"]);
  });
  it("rejects duplicates and removes by label", () => {
    const onChange = vi.fn();
    const { getByLabelText, getByRole } = render(
      <TagInput values={["a", "b"]} onChange={onChange} />,
    );
    const draft = getByLabelText("add value");
    fireEvent.change(draft, { target: { value: "a" } });
    fireEvent.keyDown(draft, { key: "Enter" });
    expect(onChange).not.toHaveBeenCalled();
    fireEvent.click(getByRole("button", { name: "remove b" }));
    expect(onChange).toHaveBeenCalledWith(["a"]);
  });
});
