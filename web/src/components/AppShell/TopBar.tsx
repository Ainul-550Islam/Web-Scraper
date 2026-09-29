// TopBar — split from AppShell.tsx; barrel re-exports keep every public path & name

import { useState } from "react";
import { Icon } from "../Icons";
import { IconButton, Segmented } from "../ui";
import { type RouteState } from "../../app/router";
import { ROUTE_BY_PATH } from "../../app/routes";
import { useTheme } from "../../hooks/useTheme";
import { useConnection } from "../../lib/connection";
import { TokenModal } from "./TokenModal";


export function TopBar({
  onOpenPalette,
  onOpenMobileNav,
  route,
}: {
  onOpenPalette: () => void;
  onOpenMobileNav: () => void;
  route: RouteState;
}) {
  const conn = useConnection();
  const { theme, setTheme } = useTheme();
  const [authOpen, setAuthOpen] = useState(false);
  const def = ROUTE_BY_PATH.get(route.path);

  return (
    <header className="topbar">
      <IconButton label="Open navigation" onClick={onOpenMobileNav} className="mobile-only">
        <Icon.Menu size={15} />
      </IconButton>
      <nav className="crumbs" aria-label="Breadcrumb">
        <span>{def?.group ?? "Console"}</span>
        <Icon.ChevronRight size={11} />
        <b>{def?.title ?? "Harvest"}</b>
      </nav>
      <div className="top-actions">
        <button className="search-launcher" onClick={onOpenPalette} aria-label="Open command palette">
          <Icon.Search size={13} />
          <span>Search or jump…</span>
          <kbd className="kbd">⌘K</kbd>
        </button>
        <Segmented
          ariaLabel="Theme"
          value={theme}
          onChange={(v) => setTheme(v)}
          options={[
            { value: "dark", label: "Dark" },
            { value: "light", label: "Light" },
            { value: "system", label: "Auto" },
          ]}
        />
        <IconButton label={conn.tokenSet ? "Bearer token set — manage" : "Set bearer token"} onClick={() => setAuthOpen(true)}>
          <Icon.Lock size={14} />
        </IconButton>
        <IconButton label="Refresh all data" onClick={conn.refreshAll}>
          <Icon.Refresh size={14} />
        </IconButton>
      </div>
      {authOpen && <TokenModal onClose={() => setAuthOpen(false)} />}
    </header>
  );
}
