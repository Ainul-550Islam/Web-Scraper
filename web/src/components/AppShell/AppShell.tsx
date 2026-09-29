// AppShell — split from AppShell.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


/** The layout frame itself (spec name). Owns the grid areas; pages are children. */
export function AppShell({
  collapsed,
  children,
}: {
  collapsed: boolean;
  children: ReactNode;
}) {
  return (
    <div className="shell" data-collapsed={collapsed ? "true" : "false"}>
      {children}
    </div>
  );
}
