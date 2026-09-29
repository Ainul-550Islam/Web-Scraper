import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { TopBar } from "../../components/AppShell/TopBar";
import { ThemeProvider } from "../../hooks/useTheme";
import { ToastProvider } from "../../hooks/useToasts";
import { ConnectionProvider } from "../../lib/connection";

function mount(route = "/runs") {
  const onOpenPalette = vi.fn();
  const onOpenMobileNav = vi.fn();
  const utils = render(
    <ThemeProvider>
      <ToastProvider>
        <ConnectionProvider>
          <TopBar
            route={{ path: route, query: new URLSearchParams() }}
            onOpenPalette={onOpenPalette}
            onOpenMobileNav={onOpenMobileNav}
          />
        </ConnectionProvider>
      </ToastProvider>
    </ThemeProvider>,
  );
  return { ...utils, onOpenPalette, onOpenMobileNav };
}

describe("TopBar (AppShell/TopBar.tsx)", () => {
  it("breadcrumbs mirror the ROUTES definition for the current page", () => {
    const { getByRole } = mount("/runs");
    const nav = getByRole("navigation", { name: "Breadcrumb" });
    expect(nav.textContent).toContain("Operations");
    expect(nav.textContent).toContain("Runs");
  });
  it("palette launcher and mobile nav button are wired to their callbacks", () => {
    const { getByRole, onOpenPalette, onOpenMobileNav } = mount();
    fireEvent.click(getByRole("button", { name: "Open command palette" }));
    fireEvent.click(getByRole("button", { name: "Open navigation" }));
    expect(onOpenPalette).toHaveBeenCalledTimes(1);
    expect(onOpenMobileNav).toHaveBeenCalledTimes(1);
  });
  it("theme is a 3-way segmented control defaulting to the persisted dark", () => {
    const { getByRole, getAllByRole } = mount();
    const group = getByRole("group", { name: "Theme" });
    const pressed = [...getAllByRole("button")].filter(
      (b) => group.contains(b) && b.getAttribute("aria-pressed") === "true",
    );
    expect(pressed.map((b) => b.textContent)).toEqual(["Dark"]);
  });
  it("token button reflects the unset state and opens the modal", async () => {
    const { getByRole } = mount();
    fireEvent.click(getByRole("button", { name: "Set bearer token" }));
    expect(await getByRole("dialog")).toBeTruthy();
  });
});
