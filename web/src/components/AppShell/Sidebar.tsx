// Sidebar — split from AppShell.tsx; barrel re-exports keep every public path & name

import { Icon } from "../Icons";
import { IconButton, StatusChip } from "../ui";
import { navigate, useRoute } from "../../app/router";
import { NAV_GROUPS, ROUTES } from "../../app/routes";
import { useConnection } from "../../lib/connection";
import { Logo } from "./Logo";
import { STATE_LABEL } from "./STATE_LABEL";


export function Sidebar({
  collapsed,
  onToggleCollapse,
  mobileOpen,
  onCloseMobile,
}: {
  collapsed: boolean;
  onToggleCollapse: () => void;
  mobileOpen: boolean;
  onCloseMobile: () => void;
}) {
  const { path } = useRoute();
  const conn = useConnection();
  const alertCount = conn.alerts?.items.length ?? 0;
  const env = conn.config?.name ?? (conn.health ? "connected" : "disconnected");

  return (
    <nav className="rail" data-open={mobileOpen ? "true" : "false"} aria-label="Primary">
      <a href="#main" className="skip-link">Skip to content</a>
      <header className="rail-head">
        <Logo />
        <span className="rail-word">Harvest</span>
        <IconButton
          label={collapsed ? "Expand navigation" : "Collapse navigation"}
          onClick={onToggleCollapse}
          style={{ marginLeft: "auto" }}
        >
          {collapsed ? <Icon.ChevronRight size={14} /> : <Icon.ChevronLeft size={14} />}
        </IconButton>
      </header>
      <div className="rail-env" title={`Engine: ${env}`}>
        <i className={`dot ${conn.health ? (conn.health.running ? "ok live" : "info") : "err"}`} aria-hidden="true" />
        <span style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{env}</span>
        <span style={{ marginLeft: "auto" }}>
          <StatusChip
            status={conn.health ? (conn.health.running ? "running" : "idle") : "unavailable"}
            dot={false}
          />
        </span>
      </div>
      <div className="rail-nav">
        {NAV_GROUPS.map((g) => (
          <div className="nav-group" key={g}>
            <div className="nav-group-title t-label">{g}</div>
            {ROUTES.filter((r) => r.group === g).map((r) => {
              const IconC = Icon[r.icon];
              const active = r.path === path;
              return (
                <button
                  key={r.path}
                  className="nav-item"
                  aria-current={active ? "page" : undefined}
                  data-tip={collapsed ? r.label : undefined}
                  onClick={() => {
                    navigate(r.path);
                    onCloseMobile();
                  }}
                >
                  <IconC size={15} />
                  <span className="nav-label">{r.label}</span>
                  {r.badge === "alerts" && alertCount > 0 && (
                    <span className="nav-badge" aria-label={`${alertCount} alerts`}>{alertCount}</span>
                  )}
                </button>
              );
            })}
          </div>
        ))}
      </div>
      <div className="rail-foot">
        <i className={`dot ${conn.state === "offline" ? "err" : "ok"}`} aria-hidden="true" />
        <span className="t-caption">{conn.base || "same-origin"} · {STATE_LABEL[conn.state]}</span>
      </div>
    </nav>
  );
}
