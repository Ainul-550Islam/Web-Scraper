import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { TextInput } from "../../components/ui";

describe("TextInput (components/ui/TextInput.tsx)", () => {
  it("controlled value + class + invalid flag, everything else passes through", () => {
    const { getByRole } = render(
      <TextInput aria-label="url" value="http://x" readOnly invalid maxLength={64} />,
    );
    const el = getByRole("textbox");
    expect(el.getAttribute("class")).toBe("input");
    expect(el.getAttribute("aria-invalid")).toBe("true");
    expect(el.getAttribute("maxlength")).toBe("64");
    expect((el as HTMLInputElement).readOnly).toBe(true);
  });
});
