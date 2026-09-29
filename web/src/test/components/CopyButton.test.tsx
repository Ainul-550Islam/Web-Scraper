import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { CopyButton } from "../../components/ui";

describe("CopyButton (components/ui/CopyButton.tsx)", () => {
  it("writes the text to the clipboard, confirms, and calls onCopied", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.defineProperty(window.navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    const onCopied = vi.fn();
    const { getByRole, findByText } = render(
      <CopyButton text="paste-me" label="Copy JSON" onCopied={onCopied} />,
    );
    fireEvent.click(getByRole("button", { name: /Copy JSON/ }));
    expect(await findByText("Copied")).toBeTruthy();
    expect(writeText).toHaveBeenCalledWith("paste-me");
    expect(onCopied).toHaveBeenCalledTimes(1);
  });
});
