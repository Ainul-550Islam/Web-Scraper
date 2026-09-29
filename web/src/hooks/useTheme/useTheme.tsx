// useTheme — split from useTheme.tsx; barrel re-exports keep every public path & name

import { useContext } from "react";
import type { ThemeCtx } from "./ThemeCtx";
import { Ctx } from "./Ctx";


export function useTheme(): ThemeCtx {
  const c = useContext(Ctx);
  if (!c) throw new Error("useTheme outside ThemeProvider");
  return c;
}
