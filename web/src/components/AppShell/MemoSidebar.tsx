// MemoSidebar — split from AppShell.tsx; barrel re-exports keep every public path & name

import { memo } from "react";
import { Sidebar } from "./Sidebar";


export const MemoSidebar = memo(Sidebar);
