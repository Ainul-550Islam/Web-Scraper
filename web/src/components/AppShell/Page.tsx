// Page — split from AppShell.tsx; barrel re-exports keep every public path & name

import { type ReactNode } from "react";


export function Page({ children }: { children: ReactNode }) {
  return <div className="main-inner">{children}</div>;
}
