// App — split from App.tsx; barrel re-exports keep every public path & name

import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import {
  AppShell,
  DemoBanner,
  MemoSidebar,
  Page,
  StatusBar,
  TopBar,
  WelcomePanel,
  ProductTour,
  markTourDone,
  tourDismissed,
  buildPaletteItems,
} from "../../components/AppShell";
import { CommandPalette, Toasts, useHotkeyPalette } from "../../components/Overlays";
import { Skeleton } from "../../components/ui";
import { PageErrorBoundary } from "../../components/PageErrorBoundary";
import { PAGE_LOADERS } from "../routes";
import { useRoute } from "../router";
import { useConnection } from "../../lib/connection";
import { useTheme } from "../../hooks/useTheme";
import { useShortcuts } from "../../hooks/useShortcuts";
import { MOCK_ENABLED } from "../../lib/mock";
import { ConnectScreen } from "../ConnectScreen";

export default function App() {
  const route = useRoute();
  const conn = useConnection();
  const { theme, setTheme, resolved } = useTheme();
  const [collapsed, setCollapsed] = useState(() => {
    try {
      return localStorage.getItem("harvest.console.nav") === "collapsed";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    try {
      localStorage.setItem("harvest.console.nav", collapsed ? "collapsed" : "expanded");
    } catch { /* storage optional */ }
  }, [collapsed]);
  const [mobileNavState, setMobileNavState] = useState<{ path: string; open: boolean }>({ path: route.path, open: false });
  const mobileNav = mobileNavState.open && mobileNavState.path === route.path;
  const closeMobileNav = () => setMobileNavState({ path: route.path, open: false });
  const [welcomeHidden, setWelcomeHidden] = useState(() => tourDismissed());
  const [tourOpen, setTourOpen] = useState(false);
  const palette = useHotkeyPalette();

  // keep the palette's theme action in sync with the *provider* (persistence
  // included), not a raw data-attribute poke
  const toggleTheme = useCallback(() => {
    setTheme(resolved === "light" ? "dark" : "light");
  }, [resolved, setTheme]);

  // "?" + system shortcut: also open via plain "/" for keyboard users
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (
        e.key === "/" &&
        !palette.open &&
        !(e.target instanceof HTMLInputElement) &&
        !(e.target instanceof HTMLTextAreaElement) &&
        !(e.target instanceof HTMLSelectElement)
      ) {
        e.preventDefault();
        palette.setOpen(true);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [palette]);

  const PageComp = PAGE_LOADERS[route.path] ?? PAGE_LOADERS["/"]!;
  const paletteItems = useMemo(
    () => buildPaletteItems(conn, toggleTheme),
    [conn, toggleTheme],
  );

  // §29 g-sequences (disabled while the palette owns the keyboard)
  useShortcuts(!palette.open);

  // §3 first-run gate: blocks the shell only if we have NEVER connected
  // this session and the transport itself is failing — not a one-off 500.
  const [gatePassed, setGatePassed] = useState(MOCK_ENABLED);
  const firstRunBlocked =
    !gatePassed && conn.health === null && conn.error?.kind === "network";
  if (firstRunBlocked) {
    return (
      <>
        <ConnectScreen
          onConnected={() => {
            setGatePassed(true);
            conn.refreshAll();
          }}
        />
        <Toasts />
        {MOCK_ENABLED && <DemoBanner demo />}
      </>
    );
  }

  return (
    <AppShell collapsed={collapsed}>
      <a className="skip-link" href="#main">Skip to content</a>
      <MemoSidebar
        collapsed={collapsed}
        onToggleCollapse={() => setCollapsed((v) => !v)}
        mobileOpen={mobileNav}
        onCloseMobile={closeMobileNav}
      />
      <TopBar
        route={route}
        onOpenPalette={() => palette.setOpen(true)}
        onOpenMobileNav={() => setMobileNavState({ path: route.path, open: true })}
      />
      <main className="main" id="main" tabIndex={-1}>
        <Page>
          {route.path === "/" && !welcomeHidden && conn.health !== null && (
            <div style={{ marginBottom: 12 }}>
              <WelcomePanel
                onStartTour={() => {
                  setWelcomeHidden(true);
                  setTourOpen(true);
                }}
                onDismiss={() => {
                  markTourDone();
                  setWelcomeHidden(true);
                }}
              />
            </div>
          )}
          <PageErrorBoundary key={route.path}>
          <Suspense
            fallback={
              <div style={{ display: "grid", gap: 12, marginTop: 8 }}>
                <Skeleton h={26} w={240} />
                <div className="grid grid-metrics">
                  {[0, 1, 2, 3].map((i) => <Skeleton key={i} h={110} />)}
                </div>
                <Skeleton h={220} />
              </div>
            }
          >
            <PageComp />
          </Suspense>
          </PageErrorBoundary>
        </Page>
      </main>
      <StatusBar route={route} />
      <Toasts />
      {tourOpen && <ProductTour open={tourOpen} onClose={() => setTourOpen(false)} />}
      {palette.open && (
        <CommandPalette items={paletteItems} onClose={() => palette.setOpen(false)} />
      )}
      <DemoBanner demo={MOCK_ENABLED} />
      {theme === "system" && <span className="sr-only">Theme follows system preference.</span>}
    </AppShell>
  );
}
