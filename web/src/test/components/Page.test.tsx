import { describe, expect, it } from "vitest";
import { render } from "@testing-library/react";
import { Page } from "../../components/AppShell";

describe("Page (AppShell/Page.tsx)", () => {
  it("every page body lands inside the .main-inner content landmark slot", () => {
    const { container } = render(<Page><h1>x</h1></Page>);
    expect(container.querySelector(".main-inner > h1")).toBeTruthy();
  });
});
