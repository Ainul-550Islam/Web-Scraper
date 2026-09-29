// buildPaletteItems — split from AppShell.tsx; barrel re-exports keep every public path & name

import { navigate } from "../../app/router";
import { ROUTES } from "../../app/routes";
import type { useConnection } from "../../lib/connection";
import type { PaletteItem } from "../Overlays";


export function buildPaletteItems(
  conn: ReturnType<typeof useConnection>,
  toggleTheme: () => void,
): PaletteItem[] {
  const nav: PaletteItem[] = ROUTES.map((r) => ({
    id: `nav-${r.path}`,
    group: "Navigate" as const,
    label: r.title,
    hint: r.label,
    run: () => navigate(r.path),
  }));
  const actions: PaletteItem[] = [
    { id: "act-refresh", group: "Actions", label: "Refresh all data", run: conn.refreshAll },
    { id: "act-theme", group: "Actions", label: "Toggle theme (dark ↔ light)", run: toggleTheme },
  ];
  // Records: only what the shell already loaded (runs/config/alerts are not
  // shell-level lists; Jobs/Proxies pages fetch their own). Kept minimal and
  // honest: targets + alerts.
  const recs: PaletteItem[] = [
    ...(conn.config?.targets ?? []).map((t) => ({
      id: `rec-t-${t.id}`, group: "Records" as const,
      label: `target: ${t.id}`, hint: t.start_url,
      run: () => navigate(`/config?target=${encodeURIComponent(t.id)}`),
    })),
    ...(conn.alerts?.items ?? []).map((a, i) => ({
      id: `rec-alert-${i}`, group: "Records" as const,
      label: `alert: ${String(a["kind"] ?? a["message"] ?? "alert")}`,
      run: () => navigate("/alerts"),
    })),
  ];
  return [...nav, ...actions, ...recs];
}
