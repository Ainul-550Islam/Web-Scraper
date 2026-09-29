import { describe, expect, it, vi } from "vitest";
import { fireEvent, render } from "@testing-library/react";
import { Sidebar } from "../../components/AppShell/Sidebar";
import { ThemeProvider } from "../../hooks/useTheme";
import { ToastProvider } from "../../hooks/useToasts";
import { ConnectionProvider } from "../../lib/connection";
import { NAV_GROUPS, ROUTES } from "../../app/routes";

function mountShell(collapsed = false) {
  const onCloseMobile = vi.fn();
  const utils = render(
    <ThemeProvider>
      <ToastProvider>
        <ConnectionProvider>
          <Sidebar
            collapsed={collapsed}
            onToggleCollapse={() => undefined}
            mobileOpen={false}
            onCloseMobile={onCloseMobile}
          />
        </ConnectionProvider>
      </ToastProvider>
    </ThemeProvider>,
  );
  return { ...utils, onCloseMobile };
}

describe("Sidebar (AppShell/Sidebar.tsx)", () => {
  it("renders every route as a nav item inside its group", () => {
    window.location.hash = "#/";
    const { getByRole, getAllByRole, container } = mountShell();
    const rail = getByRole("navigation", { name: "Primary" });
    expect(rail.getAttribute("data-open")).toBe("false");
    expect(container.querySelectorAll(".nav-item").length).toBe(ROUTES.length);
    for (const g of NAV_GROUPS) expect(rail.textContent).toContain(g);
    expect(getAllByRole("link", { name: "Skip to content" })[0]).toBeTruthy();
  });
  it("marks the active route aria-current and clicking navigates + closes mobile", () => {
    window.location.hash = "#/queue";
    const { getByRole, onCloseMobile } = mountShell();
    const queue = getByRole("button", { name: /Queue/ });
    expect(queue.getAttribute("aria-current")).toBe("page");
    fireEvent.click(getByRole("button", { name: /Runs/ }));
    expect(window.location.hash).toBe("#/runs");
    expect(onCloseMobile).toHaveBeenCalledTimes(1);
    window.location.hash = "#/";
  });
  it("collapsed rail exposes tooltips via data-tip on nav items", () => {
    window.location.hash = "#/";
    const { container } = mountShell(true);
    const item = container.querySelector(".nav-item") as HTMLElement;
    expect(item.getAttribute("data-tip")).toBeTruthy();
  });
});
