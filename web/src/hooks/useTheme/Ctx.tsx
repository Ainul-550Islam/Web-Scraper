// Ctx — split from useTheme.tsx; barrel re-exports keep every public path & name

import { createContext } from "react";
import type { ThemeCtx } from "./ThemeCtx";


export const Ctx = createContext<ThemeCtx | null>(null);
